//! Print the verification report and the change plan, and build the upload
//! batch.

use std::fs;

use super::plan::{Action, Finding, Plan, Status};
use super::sftp::{quote, ScratchDir};
use crate::error::UksftaError;
use crate::util::human_size;

pub fn print_report(plan: &Plan) {
    for report in &plan.mods {
        println!(
            "mod {} {}: {} ({} PBO(s))",
            report.id,
            report.name,
            report.status.as_str(),
            report.pbos
        );
        for finding in &report.findings {
            println!("  {}", describe(finding));
        }
    }
    for warning in &plan.warnings {
        eprintln!("Warning: {warning}");
    }
    println!(
        "{} mod(s) checked, {} with problems",
        plan.mods.len(),
        plan.problems()
    );
}

pub fn print_plan(plan: &Plan) {
    let mut any = false;
    for action in &plan.actions {
        match action {
            Action::Upload {
                local: Some(path),
                remote,
                ..
            } => {
                println!("plan: put {remote} from {}", path.display());
                any = true;
            }
            Action::Upload {
                local: None,
                remote,
                ..
            } => {
                println!("plan: put {remote} (write meta.cpp)");
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

fn describe(finding: &Finding) -> String {
    match finding {
        Finding::Missing { name, .. } => format!("missing {name}"),
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
        Finding::StaleHash { name, .. } => format!("stale {name}: contents differ"),
        Finding::IdMismatch {
            id,
            remote: Some(remote),
        } => format!("meta.cpp publishedid is {remote}, expected {id}"),
        Finding::IdMismatch { id, remote: None } => {
            format!("meta.cpp is missing or has no publishedid, expected {id}")
        }
        Finding::Extra { name, .. } => format!("extra {name}"),
        Finding::NoSignature { name, .. } => format!("[NO SIGNATURE] {name}"),
        Finding::NoDirectory { id } => format!("no mod folder @{id} on the server"),
        Finding::NoLock { id } => format!("no mods.lock entry for {id}"),
    }
}

/// The upload phase: create a missing mod directory, then put every file.
pub fn upload_batch(plan: &Plan, scratch: &ScratchDir) -> Result<Vec<String>, UksftaError> {
    let mut dirs: Vec<String> = Vec::new();
    let mut puts = Vec::new();
    let missing: Vec<&str> = plan
        .mods
        .iter()
        .filter(|report| report.status == Status::NoDirectory)
        .map(|report| report.id.as_str())
        .collect();

    for action in plan.uploads() {
        if let Action::Upload {
            id, local, remote, ..
        } = action
        {
            if missing.contains(&id.as_str()) {
                if let Some((dir, _)) = remote.rsplit_once('/') {
                    if !dirs.iter().any(|known| known == dir) {
                        dirs.push(dir.to_string());
                    }
                }
            }
            let source = match local {
                Some(path) => path.clone(),
                None => {
                    let path = scratch.join(&format!("{id}-meta.cpp"));
                    let name = plan
                        .mods
                        .iter()
                        .find(|report| &report.id == id)
                        .map_or(id.as_str(), |report| report.name.as_str());
                    fs::write(&path, generated_meta_cpp(id, name)).map_err(UksftaError::Io)?;
                    path
                }
            };
            puts.push(format!(
                "put {} {}",
                quote(&source.display().to_string())?,
                quote(remote)?
            ));
        }
    }

    dirs.sort_by_key(|dir| dir.matches('/').count());
    let mut batch = Vec::new();
    for dir in dirs {
        batch.push(format!("-mkdir {}", quote(&dir)?));
    }
    batch.extend(puts);
    Ok(batch)
}

/// The minimal `meta.cpp`, in the format the server writes. A cache copy is
/// preferred, so this is the fallback only.
fn generated_meta_cpp(id: &str, name: &str) -> String {
    format!(
        "protocol = 1;\npublishedid = {id};\nname = \"{}\";\n",
        cpp_string(name)
    )
}

fn cpp_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_meta_cpp_uses_the_server_format() {
        let text = generated_meta_cpp("450814997", "CBA \"A3\"");
        assert!(text.contains("protocol = 1;"));
        assert!(text.contains("publishedid = 450814997;"));
        assert!(text.contains("name = \"CBA \\\"A3\\\"\";"));
        assert!(!text.contains("protocolVersion"));
    }
}
