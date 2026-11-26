//! Print the pack report and the change plan, and build the upload batch.

use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;

use super::plan::{inside, Action, Finding, Plan};
use super::sftp::quote;
use crate::error::UksftaError;
use crate::util::human_size;

pub fn print_report(plan: &Plan, out: &mut dyn Write) {
    let _ = writeln!(
        out,
        "local {}: {} PBO(s), {} signature(s), {} key(s)",
        plan.local_dir.display(),
        plan.local_pbos,
        plan.local_signatures,
        plan.local_keys
    );
    if plan.remote_exists {
        let _ = writeln!(
            out,
            "remote {}: {} PBO(s), {} key(s)",
            plan.remote_dir, plan.remote_pbos, plan.remote_keys
        );
    } else {
        let _ = writeln!(out, "remote {}: no pack folder", plan.remote_dir);
    }
    let _ = writeln!(out, "status: {}", plan.status().as_str());
    for finding in &plan.findings {
        let _ = writeln!(out, "  {}", describe(finding, &plan.id));
    }
    if plan.unsigned > 0 {
        let _ = writeln!(
            out,
            "unsigned: {} of {} PBO(s) have no signature (advisory)",
            plan.unsigned, plan.local_pbos
        );
    }
    for warning in &plan.warnings {
        eprintln!("Warning: {warning}");
    }
    let _ = writeln!(
        out,
        "PBOs: {} local, {} local-only, {} remote-only",
        plan.local_pbos,
        plan.local_only(),
        plan.remote_only()
    );
}

fn describe(finding: &Finding, pack_id: &str) -> String {
    match finding {
        Finding::NoPack => "no pack folder on the server".to_string(),
        Finding::MetaMismatch {
            remote: Some(remote),
        } => format!("meta.cpp publishedid is {remote}, expected {pack_id}"),
        Finding::MetaMismatch { remote: None } => {
            "meta.cpp is missing or has no publishedid".to_string()
        }
        Finding::Missing { name } => format!("missing {name}"),
        Finding::StaleSize {
            name,
            local,
            remote,
            ..
        } => format!(
            "stale {name}: local {}, remote {}",
            human_size(*local),
            human_size(*remote)
        ),
        Finding::StaleHash { name } => format!("stale {name}: contents differ"),
        Finding::Extra { name } => format!("extra {name}"),
        Finding::KeyMissing { name } => format!("missing key {name}"),
        Finding::KeyStale {
            name,
            local,
            remote,
            ..
        } => format!(
            "stale key {name}: local {}, remote {}",
            human_size(*local),
            human_size(*remote)
        ),
        Finding::KeyExtra { name } => format!("extra key {name}"),
        Finding::KeyAuthorityMissing {
            signature,
            authority,
        } => format!("signature {signature} needs key {authority}, which keys/ does not ship"),
        Finding::ModCppMissing => "mod.cpp is missing".to_string(),
        Finding::ModCppStale { local, remote } => format!(
            "stale mod.cpp: local {}, remote {}",
            human_size(*local),
            human_size(*remote)
        ),
    }
}

pub fn print_plan(plan: &Plan, out: &mut dyn Write) {
    let mut any = false;
    for action in &plan.actions {
        match action {
            Action::Upload { local, remote, .. } => {
                let _ = writeln!(out, "plan: put {remote} from {}", local.display());
                any = true;
            }
            Action::Remove { remote } => {
                let _ = writeln!(out, "plan: rm {remote}");
                any = true;
            }
        }
    }
    if !any {
        let _ = writeln!(out, "plan: no changes");
    }
}

/// One file to upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadUnit {
    pub local: PathBuf,
    pub remote: String,
    pub size: u64,
}

/// The files to upload, smallest first.
pub fn upload_units(plan: &Plan) -> Vec<UploadUnit> {
    let mut units: Vec<UploadUnit> = plan
        .uploads()
        .filter_map(|action| match action {
            Action::Upload {
                local,
                remote,
                size,
                ..
            } => Some(UploadUnit {
                local: local.clone(),
                remote: remote.clone(),
                size: *size,
            }),
            Action::Remove { .. } => None,
        })
        .collect();
    sort_ascending(&mut units);
    units
}

fn sort_ascending(units: &mut [UploadUnit]) {
    units.sort_by(|a, b| a.size.cmp(&b.size).then_with(|| a.remote.cmp(&b.remote)));
}

/// The `-mkdir` lines for the directories the uploads need, starting with
/// the pack folder, then ordered by depth and name.
///
/// # Errors
/// Returns `Input` when a remote path cannot be quoted or escapes the
/// pack folder.
pub fn upload_dirs(plan: &Plan) -> Result<Vec<String>, UksftaError> {
    let prefix = format!("{}/", plan.remote_dir);
    let mut rels: Vec<String> = Vec::new();
    let mut any = false;
    for unit in upload_units(plan) {
        any = true;
        let rel = unit.remote.strip_prefix(&prefix).ok_or_else(|| {
            UksftaError::Input(format!(
                "upload target outside the pack folder: {}",
                unit.remote
            ))
        })?;
        let mut parts: Vec<&str> = rel.split('/').collect();
        parts.pop();
        let mut acc = String::new();
        for part in parts {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(part);
            if !rels.contains(&acc) {
                rels.push(acc.clone());
            }
        }
    }

    let mut dirs = Vec::new();
    if any {
        dirs.push(plan.remote_dir.clone());
        for rel in rels {
            dirs.push(inside(&plan.remote_dir, &rel)?);
        }
    }
    dirs.sort_by(|a, b| {
        a.matches('/')
            .count()
            .cmp(&b.matches('/').count())
            .then_with(|| a.cmp(b))
    });
    dirs.dedup();

    let mut lines = Vec::new();
    for dir in &dirs {
        lines.push(format!("-mkdir {}", quote(dir)?));
    }
    Ok(lines)
}

/// One `put -p` line. The server keeps the local modification time.
///
/// # Errors
/// Returns `Input` when a path cannot be quoted.
pub fn put_line(unit: &UploadUnit) -> Result<String, UksftaError> {
    Ok(format!(
        "put -p {} {}",
        quote(&unit.local.display().to_string())?,
        quote(&unit.remote)?
    ))
}

/// Split the units across workers by greedy longest-processing-time: the
/// largest file goes first, each to the worker with the smallest total.
/// A tie goes to the lowest worker index. Each worker is then ascending.
pub fn plan_workers(units: Vec<UploadUnit>, jobs: u8) -> Vec<Vec<UploadUnit>> {
    let worker_count = usize::from(jobs.max(1));
    if worker_count == 1 {
        let mut only = units;
        sort_ascending(&mut only);
        return vec![only];
    }

    let mut units = units;
    units.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.remote.cmp(&b.remote)));
    let mut buckets: Vec<Vec<UploadUnit>> = vec![Vec::new(); worker_count];
    let mut totals = vec![0u64; worker_count];
    for unit in units {
        let target = smallest_total(&totals);
        totals[target] = totals[target].saturating_add(unit.size);
        buckets[target].push(unit);
    }
    buckets.retain(|bucket| !bucket.is_empty());
    for bucket in &mut buckets {
        sort_ascending(bucket);
    }
    buckets
}

/// The index of the worker with the smallest total. The lowest index wins.
fn smallest_total(totals: &[u64]) -> usize {
    let mut index = 0;
    for (candidate, total) in totals.iter().enumerate().skip(1) {
        if *total < totals[index] {
            index = candidate;
        }
    }
    index
}

/// The upload phase. Only `addons/` and `keys/` are created. The puts run
/// smallest first, so a failure in the middle leaves the large files for a
/// retry. A remote path in `skip` is left out; the segmented path sends it.
///
/// # Errors
/// Returns `Input` when a path cannot be quoted.
pub fn upload_batch(plan: &Plan, skip: &HashSet<String>) -> Result<Vec<String>, UksftaError> {
    let mut batch = upload_dirs(plan)?;
    for unit in upload_units(plan) {
        if skip.contains(&unit.remote) {
            continue;
        }
        batch.push(put_line(&unit)?);
    }
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn upload(name: &str, remote: &str, size: u64) -> Action {
        Action::Upload {
            name: name.to_string(),
            local: PathBuf::from(format!("/local/{name}")),
            remote: remote.to_string(),
            size,
        }
    }

    fn plan_with(actions: Vec<Action>) -> Plan {
        Plan {
            remote_dir: "/srv/arma/@1".to_string(),
            actions,
            ..Plan::default()
        }
    }

    #[test]
    fn the_upload_batch_puts_the_smallest_file_first() {
        let plan = plan_with(vec![
            upload("big.pbo", "/srv/arma/@1/addons/big.pbo", 900),
            upload("key.bikey", "/srv/arma/@1/keys/key.bikey", 50),
            upload("small.pbo", "/srv/arma/@1/addons/small.pbo", 100),
        ]);

        let batch = upload_batch(&plan, &HashSet::new()).unwrap();
        let puts: Vec<&String> = batch
            .iter()
            .filter(|line| line.starts_with("put "))
            .collect();
        assert_eq!(puts.len(), 3);
        assert!(puts[0].contains("key.bikey"), "{batch:?}");
        assert!(puts[1].contains("small.pbo"), "{batch:?}");
        assert!(puts[2].contains("big.pbo"), "{batch:?}");
        assert_eq!(
            batch[0], "-mkdir \"/srv/arma/@1\"",
            "the pack folder comes first: {batch:?}"
        );
        assert!(batch[1].starts_with("-mkdir"), "{batch:?}");
        assert!(batch[2].starts_with("-mkdir"), "{batch:?}");
        assert!(batch[3].starts_with("put "), "{batch:?}");
    }

    #[test]
    fn a_nested_upload_creates_its_directory_once_before_the_put() {
        let plan = plan_with(vec![
            upload("x.py", "/srv/arma/@1/python_code/x.py", 10),
            upload("y.py", "/srv/arma/@1/python_code/y.py", 20),
        ]);

        let batch = upload_batch(&plan, &HashSet::new()).unwrap();
        let mkdir = batch
            .iter()
            .position(|line| line == "-mkdir \"/srv/arma/@1/python_code\"")
            .expect("the nested directory is created");
        let put = batch
            .iter()
            .position(|line| line.starts_with("put "))
            .expect("the files are sent");
        assert!(mkdir < put, "{batch:?}");
        assert_eq!(
            batch
                .iter()
                .filter(|line| line.contains("python_code"))
                .filter(|line| line.starts_with("-mkdir"))
                .count(),
            1,
            "{batch:?}"
        );
    }

    #[test]
    fn every_put_preserves_the_timestamp() {
        let plan = plan_with(vec![upload("a.pbo", "/srv/arma/@1/addons/a.pbo", 10)]);

        let batch = upload_batch(&plan, &HashSet::new()).unwrap();
        let puts: Vec<&String> = batch
            .iter()
            .filter(|line| line.starts_with("put "))
            .collect();
        assert_eq!(puts.len(), 1);
        assert!(puts[0].starts_with("put -p \""), "{batch:?}");
    }

    fn sample_plan() -> Plan {
        plan_with(vec![
            upload("big.pbo", "/srv/arma/@1/addons/big.pbo", 900),
            upload("mid.pbo", "/srv/arma/@1/addons/mid.pbo", 400),
            upload("key.bikey", "/srv/arma/@1/keys/key.bikey", 50),
            upload("small.pbo", "/srv/arma/@1/addons/small.pbo", 100),
            upload("sig.bisign", "/srv/arma/@1/addons/sig.bisign", 70),
        ])
    }

    #[test]
    fn one_job_keeps_every_unit_in_one_worker_smallest_first() {
        let workers = plan_workers(upload_units(&sample_plan()), 1);

        assert_eq!(workers.len(), 1);
        let sizes: Vec<u64> = workers[0].iter().map(|unit| unit.size).collect();
        assert_eq!(sizes, vec![50, 70, 100, 400, 900]);
    }

    #[test]
    fn three_jobs_partition_every_unit_exactly_once() {
        let units = upload_units(&sample_plan());
        let expected_bytes: u64 = units.iter().map(|unit| unit.size).sum();
        let mut expected: Vec<String> = units.iter().map(|unit| unit.remote.clone()).collect();
        expected.sort();

        let workers = plan_workers(units, 3);
        let mut got: Vec<String> = workers
            .iter()
            .flatten()
            .map(|unit| unit.remote.clone())
            .collect();
        got.sort();

        assert_eq!(got, expected);
        assert_eq!(
            workers.iter().flatten().map(|unit| unit.size).sum::<u64>(),
            expected_bytes
        );
    }

    #[test]
    fn worker_totals_stay_within_the_largest_file() {
        let units = upload_units(&sample_plan());
        let largest = units.iter().map(|unit| unit.size).max().unwrap_or(0);
        let workers = plan_workers(units, 3);
        let totals: Vec<u64> = workers
            .iter()
            .map(|worker| worker.iter().map(|unit| unit.size).sum())
            .collect();
        let max = totals.iter().max().copied().unwrap_or(0);
        let min = totals.iter().min().copied().unwrap_or(0);
        assert!(max - min <= largest, "{totals:?}");
    }

    #[test]
    fn every_worker_is_ascending() {
        let workers = plan_workers(upload_units(&sample_plan()), 3);

        for worker in &workers {
            let sizes: Vec<u64> = worker.iter().map(|unit| unit.size).collect();
            let mut sorted = sizes.clone();
            sorted.sort_unstable();
            assert_eq!(sizes, sorted, "{worker:?}");
        }
    }
}
