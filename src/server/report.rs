//! Print the pack report and the change plan, and build the upload batch.

use std::path::Path;

use super::plan::{Action, Finding, Plan};
use super::sftp::quote;
use crate::error::UksftaError;
use crate::util::human_size;

pub fn print_report(plan: &Plan) {
    println!(
        "local {}: {} PBO(s), {} signature(s), {} key(s)",
        plan.local_dir.display(),
        plan.local_pbos,
        plan.local_signatures,
        plan.local_keys
    );
    if plan.remote_exists {
        println!(
            "remote {}: {} PBO(s), {} key(s)",
            plan.remote_dir, plan.remote_pbos, plan.remote_keys
        );
    } else {
        println!("remote {}: no pack folder", plan.remote_dir);
    }
    println!("status: {}", plan.status().as_str());
    for finding in &plan.findings {
        println!("  {}", describe(finding, &plan.id));
    }
    if plan.unsigned > 0 {
        println!(
            "unsigned: {} of {} PBO(s) have no signature (advisory)",
            plan.unsigned, plan.local_pbos
        );
    }
    for warning in &plan.warnings {
        eprintln!("Warning: {warning}");
    }
    println!(
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
        Finding::ModCppMissing => "mod.cpp is missing".to_string(),
        Finding::ModCppStale { local, remote } => format!(
            "stale mod.cpp: local {}, remote {}",
            human_size(*local),
            human_size(*remote)
        ),
    }
}

pub fn print_plan(plan: &Plan) {
    let mut any = false;
    for action in &plan.actions {
        match action {
            Action::Upload { local, remote, .. } => {
                println!("plan: put {remote} from {}", local.display());
                any = true;
            }
            Action::Remove { remote } => {
                println!("plan: rm {remote}");
                any = true;
            }
        }
    }
    if !any {
        println!("plan: no changes");
    }
}

/// The upload phase. Only `addons/` and `keys/` are created. The puts run
/// smallest first, so a failure in the middle leaves the large files for a
/// retry.
pub fn upload_batch(plan: &Plan) -> Result<Vec<String>, UksftaError> {
    let addons = format!("{}/addons", plan.remote_dir);
    let keys = format!("{}/keys", plan.remote_dir);
    let mut dirs: Vec<String> = Vec::new();
    let mut uploads: Vec<(&u64, &Path, &str)> = plan
        .uploads()
        .filter_map(|action| match action {
            Action::Upload {
                local,
                remote,
                size,
                ..
            } => Some((size, local.as_path(), remote.as_str())),
            Action::Remove { .. } => None,
        })
        .collect();
    uploads.sort_by_key(|(size, _, _)| **size);

    let mut puts = Vec::new();
    for (_, local, remote) in uploads {
        if remote.starts_with(&format!("{addons}/")) && !dirs.contains(&addons) {
            dirs.push(addons.clone());
        }
        if remote.starts_with(&format!("{keys}/")) && !dirs.contains(&keys) {
            dirs.push(keys.clone());
        }
        puts.push(format!(
            "put -p {} {}",
            quote(&local.display().to_string())?,
            quote(remote)?
        ));
    }

    dirs.sort();
    let mut batch = Vec::new();
    for dir in &dirs {
        batch.push(format!("-mkdir {}", quote(dir)?));
    }
    batch.extend(puts);
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

        let batch = upload_batch(&plan).unwrap();
        let puts: Vec<&String> = batch
            .iter()
            .filter(|line| line.starts_with("put "))
            .collect();
        assert_eq!(puts.len(), 3);
        assert!(puts[0].contains("key.bikey"), "{batch:?}");
        assert!(puts[1].contains("small.pbo"), "{batch:?}");
        assert!(puts[2].contains("big.pbo"), "{batch:?}");
        assert!(batch[0].starts_with("-mkdir"), "{batch:?}");
        assert!(batch[1].starts_with("-mkdir"), "{batch:?}");
        assert!(batch[2].starts_with("put "), "{batch:?}");
    }

    #[test]
    fn every_put_preserves_the_timestamp() {
        let plan = plan_with(vec![upload("a.pbo", "/srv/arma/@1/addons/a.pbo", 10)]);

        let batch = upload_batch(&plan).unwrap();
        let puts: Vec<&String> = batch
            .iter()
            .filter(|line| line.starts_with("put "))
            .collect();
        assert_eq!(puts.len(), 1);
        assert!(puts[0].starts_with("put -p \""), "{batch:?}");
    }
}
