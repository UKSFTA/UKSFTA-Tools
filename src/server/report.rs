//! Print the pack report and the change plan, and build the upload batch.

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

/// The upload phase. Only `addons/` and `keys/` are created.
pub fn upload_batch(plan: &Plan) -> Result<Vec<String>, UksftaError> {
    let addons = format!("{}/addons", plan.remote_dir);
    let keys = format!("{}/keys", plan.remote_dir);
    let mut dirs: Vec<String> = Vec::new();
    let mut puts = Vec::new();

    for action in plan.uploads() {
        if let Action::Upload { local, remote, .. } = action {
            if remote.starts_with(&format!("{addons}/")) && !dirs.contains(&addons) {
                dirs.push(addons.clone());
            }
            if remote.starts_with(&format!("{keys}/")) && !dirs.contains(&keys) {
                dirs.push(keys.clone());
            }
            puts.push(format!(
                "put {} {}",
                quote(&local.display().to_string())?,
                quote(remote)?
            ));
        }
    }

    dirs.sort();
    let mut batch = Vec::new();
    for dir in &dirs {
        batch.push(format!("-mkdir {}", quote(dir)?));
    }
    batch.extend(puts);
    Ok(batch)
}
