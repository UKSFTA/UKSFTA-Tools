//! Build the plan by comparing the local build with the remote pack folder.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::join_remote;
use super::model::{is_bikey, is_pbo, Action, Finding, LocalFile, Plan, PlanInput, RemoteFile};

/// The PBO that a signature belongs to:
/// `uksfta_main.pbo.uksfta_1.0.1.bisign` -> `uksfta_main.pbo`.
pub(super) fn signature_owner(name: &str) -> Option<&str> {
    let lower = name.to_ascii_lowercase();
    if !lower.ends_with(".bisign") {
        return None;
    }
    let stem = &name[..name.len() - ".bisign".len()];
    let pos = stem.to_ascii_lowercase().find(".pbo.")?;
    Some(&stem[..pos + 4])
}

/// True when `signature` belongs to `pbo`, ignoring case.
pub(super) fn is_signature_of(signature: &str, pbo: &str) -> bool {
    signature_owner(signature).is_some_and(|owner| owner.eq_ignore_ascii_case(pbo))
}

fn lower(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// Build the plan.
pub fn build_plan(input: &PlanInput<'_>) -> Plan {
    let local = input.local;
    let remote = input.remote;
    let mut findings = Vec::new();
    let mut warnings = Vec::new();
    let mut unsigned = 0;
    let mut case_differences = 0;
    let mut queue = Queue::default();

    if !remote.exists {
        findings.push(Finding::NoPack);
    }

    let addons_dir = join_remote(input.remote_dir, "addons");
    let keys_dir = join_remote(input.remote_dir, "keys");
    let addons: HashMap<String, &RemoteFile> = remote
        .addons
        .iter()
        .map(|file| (lower(&file.name), file))
        .collect();
    let keys: HashMap<String, &RemoteFile> = remote
        .keys
        .iter()
        .map(|file| (lower(&file.name), file))
        .collect();
    let addons_links: HashSet<String> = remote.addons_symlinks.iter().map(|n| lower(n)).collect();
    let keys_links: HashSet<String> = remote.keys_symlinks.iter().map(|n| lower(n)).collect();

    // The remote meta.cpp must name this pack.
    let meta_matches = remote.meta_id.as_deref().is_some_and(|id| id == local.id);
    if !meta_matches {
        findings.push(Finding::MetaMismatch {
            remote: remote.meta_id.clone(),
        });
        match input.meta_upload {
            Some(path) => queue.upload("meta.cpp", path, join_remote(input.remote_dir, "meta.cpp")),
            None => warnings.push("meta.cpp: no local copy to upload".to_string()),
        }
    }

    // mod.cpp is compared by size only.
    match (&local.mod_cpp, remote.mod_size) {
        (Some(local_mod), None) => {
            findings.push(Finding::ModCppMissing);
            queue.upload(
                "mod.cpp",
                &local_mod.path,
                join_remote(input.remote_dir, "mod.cpp"),
            );
        }
        (Some(local_mod), Some(size)) if size != local_mod.size => {
            findings.push(Finding::ModCppStale {
                local: local_mod.size,
                remote: size,
            });
            queue.upload(
                "mod.cpp",
                &local_mod.path,
                join_remote(input.remote_dir, "mod.cpp"),
            );
        }
        (Some(_), Some(_)) => {}
        (None, Some(_)) => warnings.push("mod.cpp: no local copy to upload".to_string()),
        (None, None) => {}
    }

    for pbo in &local.pbos {
        let key = lower(&pbo.name);
        let found = addons.get(&key).copied();
        if let Some(file) = found {
            if file.name != pbo.name {
                case_differences += 1;
            }
        }
        let target = found.map_or(pbo.name.clone(), |file| file.name.clone());

        let mut stale = false;
        match found {
            None => {
                findings.push(Finding::Missing {
                    name: pbo.name.clone(),
                });
                stale = true;
            }
            Some(file) if file.size != pbo.size => {
                findings.push(Finding::StaleSize {
                    name: pbo.name.clone(),
                    local: pbo.size,
                    remote: file.size,
                });
                stale = true;
            }
            Some(_) if input.deep.get(&key) == Some(&false) => {
                findings.push(Finding::StaleHash {
                    name: pbo.name.clone(),
                });
                stale = true;
            }
            Some(_) => {}
        }

        let paired: Vec<&LocalFile> = local
            .signatures
            .iter()
            .filter(|signature| is_signature_of(&signature.name, &pbo.name))
            .collect();
        if paired.is_empty() {
            unsigned += 1;
        }

        let blocked = addons_links.contains(&key);
        if blocked {
            warnings.push(format!("{}: symlink on the server; not written", pbo.name));
            continue;
        }
        if stale {
            queue.upload(&pbo.name, &pbo.path, join_remote(&addons_dir, &target));
        }
        for signature in paired {
            let sig_key = lower(&signature.name);
            let remote_signature = addons.get(&sig_key).copied();
            let sig_target =
                remote_signature.map_or(signature.name.clone(), |file| file.name.clone());
            let needed = stale || remote_signature.is_none_or(|file| file.size != signature.size);
            if needed {
                queue.upload(
                    &signature.name,
                    &signature.path,
                    join_remote(&addons_dir, &sig_target),
                );
            }
        }
    }

    // Remote PBOs with no local match are extras.
    let local_pbos: HashSet<String> = local.pbos.iter().map(|p| lower(&p.name)).collect();
    for file in &remote.addons {
        if !is_pbo(&file.name) || local_pbos.contains(&lower(&file.name)) {
            continue;
        }
        findings.push(Finding::Extra {
            name: file.name.clone(),
        });
        if input.locked.contains(&lower(&file.name)) && !input.force_prune {
            warnings.push(format!("{}: listed in mods.lock; not pruned", file.name));
            continue;
        }
        queue.remove(join_remote(&addons_dir, &file.name));
    }

    // Keys are compared by name and size, and never removed.
    let local_keys: HashSet<String> = local.keys.iter().map(|k| lower(&k.name)).collect();
    for key in &local.keys {
        let name = lower(&key.name);
        match keys.get(&name) {
            None => {
                findings.push(Finding::KeyMissing {
                    name: key.name.clone(),
                });
                if !keys_links.contains(&name) {
                    queue.upload(&key.name, &key.path, join_remote(&keys_dir, &key.name));
                }
            }
            Some(file) => {
                if file.size != key.size {
                    findings.push(Finding::KeyStale {
                        name: key.name.clone(),
                        local: key.size,
                        remote: file.size,
                    });
                    if !keys_links.contains(&name) {
                        queue.upload(&key.name, &key.path, join_remote(&keys_dir, &file.name));
                    }
                }
            }
        }
    }
    for file in &remote.keys {
        if !is_bikey(&file.name) || local_keys.contains(&lower(&file.name)) {
            continue;
        }
        findings.push(Finding::KeyExtra {
            name: file.name.clone(),
        });
    }

    if case_differences > 0 {
        warnings.push(format!(
            "{case_differences} PBO name(s) differ only by case; uploads reuse the server name"
        ));
    }

    Plan {
        id: local.id.clone(),
        local_dir: local.dir.clone(),
        remote_dir: input.remote_dir.to_string(),
        remote_exists: remote.exists,
        local_pbos: local.pbos.len(),
        local_signatures: local.signatures.len(),
        local_keys: local.keys.len(),
        remote_pbos: remote.addons.iter().filter(|f| is_pbo(&f.name)).count(),
        remote_keys: remote.keys.len(),
        findings,
        actions: queue.actions,
        warnings,
        unsigned,
        require_signatures: input.require_signatures,
    }
}

/// Collect actions once per remote path.
#[derive(Default)]
struct Queue {
    actions: Vec<Action>,
    queued: HashSet<String>,
}

impl Queue {
    fn upload(&mut self, name: &str, local: &Path, remote: String) {
        if self.queued.insert(remote.clone()) {
            self.actions.push(Action::Upload {
                name: name.to_string(),
                local: local.to_path_buf(),
                remote,
            });
        }
    }

    fn remove(&mut self, remote: String) {
        if self.queued.insert(remote.clone()) {
            self.actions.push(Action::Remove { remote });
        }
    }
}
