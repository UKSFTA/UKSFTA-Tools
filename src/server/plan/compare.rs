//! Build the plan by comparing the local build with the remote pack folder.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::model::{
    display_name, is_addons_pbo, is_addons_signature, is_key_file, Action, Finding, Plan,
    PlanInput, RemoteEntry,
};
use super::tree::inside;

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

fn basename(rel: &str) -> &str {
    rel.rsplit_once('/').map_or(rel, |(_, name)| name)
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

    let remote_by_rel: HashMap<String, &RemoteEntry> = remote
        .files
        .iter()
        .map(|entry| (lower(&entry.rel), entry))
        .collect();
    let remote_exact: HashMap<&str, &RemoteEntry> = remote
        .files
        .iter()
        .map(|entry| (entry.rel.as_str(), entry))
        .collect();
    let remote_links: HashSet<String> = remote.symlinks.iter().map(|rel| lower(rel)).collect();
    let remote_signatures: HashSet<String> =
        remote.signatures().map(|entry| lower(&entry.rel)).collect();

    // The remote meta.cpp must name this pack.
    let meta_matches = remote.meta_id.as_deref().is_some_and(|id| id == local.id);
    if !meta_matches {
        findings.push(Finding::MetaMismatch {
            remote: remote.meta_id.clone(),
        });
        match input.meta_upload {
            Some(path) => queue.upload(
                "meta.cpp",
                path,
                input.remote_dir,
                "meta.cpp",
                0,
                &mut warnings,
            ),
            None => warnings.push("meta.cpp: no local copy to upload".to_string()),
        }
    }

    // mod.cpp is compared by size only.
    match (local.mod_cpp(), remote.mod_size) {
        (Some(local_mod), None) => {
            findings.push(Finding::ModCppMissing);
            queue.upload(
                "mod.cpp",
                &local_mod.path,
                input.remote_dir,
                "mod.cpp",
                local_mod.size,
                &mut warnings,
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
                input.remote_dir,
                "mod.cpp",
                local_mod.size,
                &mut warnings,
            );
        }
        (Some(_), Some(_)) => {}
        (None, Some(_)) => warnings.push("mod.cpp: no local copy to upload".to_string()),
        (None, None) => {}
    }

    let signatures: Vec<&super::tree::LocalEntry> = local.signatures().collect();
    for pbo in local.pbos() {
        let key = lower(&pbo.rel);
        let found = remote_by_rel.get(&key).copied();
        if let Some(file) = found {
            if file.rel != pbo.rel {
                case_differences += 1;
            }
        }
        let target = found.map_or(pbo.rel.clone(), |file| file.rel.clone());

        let mut stale = false;
        match found {
            None => {
                findings.push(Finding::Missing {
                    name: display_name(&pbo.rel),
                });
                stale = true;
            }
            Some(file) if file.size != pbo.size => {
                findings.push(Finding::StaleSize {
                    name: display_name(&pbo.rel),
                    local: pbo.size,
                    remote: file.size,
                });
                stale = true;
            }
            Some(_) if input.deep.get(&key) == Some(&false) => {
                findings.push(Finding::StaleHash {
                    name: display_name(&pbo.rel),
                });
                stale = true;
            }
            Some(_) => {}
        }

        let paired: Vec<&&super::tree::LocalEntry> = signatures
            .iter()
            .filter(|signature| is_signature_of(basename(&signature.rel), basename(&pbo.rel)))
            .collect();
        if paired.is_empty() {
            unsigned += 1;
        }

        let blocked = remote_links.contains(&key);
        if blocked {
            warnings.push(format!(
                "{}: symlink on the server; not written",
                display_name(&pbo.rel)
            ));
            continue;
        }
        if stale {
            queue.upload(
                &display_name(&pbo.rel),
                &pbo.path,
                input.remote_dir,
                &target,
                pbo.size,
                &mut warnings,
            );
        }
        for signature in paired {
            let sig_key = lower(&signature.rel);
            let remote_signature = remote_by_rel.get(&sig_key).copied();
            let sig_target =
                remote_signature.map_or(signature.rel.clone(), |file| file.rel.clone());
            let needed = stale || remote_signature.is_none_or(|file| file.size != signature.size);
            if needed {
                queue.upload(
                    &display_name(&signature.rel),
                    &signature.path,
                    input.remote_dir,
                    &sig_target,
                    signature.size,
                    &mut warnings,
                );
            }
        }
    }

    // Remote PBOs with no local match are extras and the only prune target.
    let local_rels: HashSet<String> = local.files.iter().map(|file| lower(&file.rel)).collect();
    let local_exact: HashSet<&str> = local.files.iter().map(|file| file.rel.as_str()).collect();
    for file in &remote.files {
        let special = is_addons_pbo(&file.rel)
            || is_key_file(&file.rel)
            || remote_signatures.contains(&lower(&file.rel))
            || file.rel == "meta.cpp"
            || file.rel == "mod.cpp";
        if !special {
            // Included files are reported by exact path, never removed.
            if !local_exact.contains(file.rel.as_str()) {
                findings.push(Finding::Extra {
                    name: display_name(&file.rel),
                });
            }
            continue;
        }
        if local_rels.contains(&lower(&file.rel)) {
            continue;
        }
        if is_addons_pbo(&file.rel) {
            findings.push(Finding::Extra {
                name: display_name(&file.rel),
            });
            if input.locked.contains(&lower(basename(&file.rel))) && !input.force_prune {
                warnings.push(format!(
                    "{}: listed in mods.lock; not pruned",
                    display_name(&file.rel)
                ));
                continue;
            }
            queue.remove(input.remote_dir, &file.rel, &mut warnings);
        } else if is_key_file(&file.rel) {
            findings.push(Finding::KeyExtra {
                name: display_name(&file.rel),
            });
        } else {
            continue;
        }
    }

    // Keys are compared by name and size, and never removed.
    for key in local.keys() {
        let rel_key = lower(&key.rel);
        match remote_by_rel.get(&rel_key).copied() {
            None => {
                findings.push(Finding::KeyMissing {
                    name: display_name(&key.rel),
                });
                if !remote_links.contains(&rel_key) {
                    queue.upload(
                        &display_name(&key.rel),
                        &key.path,
                        input.remote_dir,
                        &key.rel,
                        key.size,
                        &mut warnings,
                    );
                }
            }
            Some(file) => {
                if file.size != key.size {
                    findings.push(Finding::KeyStale {
                        name: display_name(&key.rel),
                        local: key.size,
                        remote: file.size,
                    });
                    if !remote_links.contains(&rel_key) {
                        queue.upload(
                            &display_name(&key.rel),
                            &key.path,
                            input.remote_dir,
                            &file.rel,
                            key.size,
                            &mut warnings,
                        );
                    }
                }
            }
        }
    }

    // Every other included file is compared by relative path.
    for file in &local.files {
        if file.rel == "meta.cpp"
            || file.rel == "mod.cpp"
            || is_addons_pbo(&file.rel)
            || is_addons_signature(&file.rel)
            || is_key_file(&file.rel)
        {
            continue;
        }
        match remote_exact.get(file.rel.as_str()).copied() {
            None => {
                findings.push(Finding::Missing {
                    name: display_name(&file.rel),
                });
                queue.upload(
                    &display_name(&file.rel),
                    &file.path,
                    input.remote_dir,
                    &file.rel,
                    file.size,
                    &mut warnings,
                );
            }
            Some(remote_file) if remote_file.size != file.size => {
                findings.push(Finding::StaleSize {
                    name: display_name(&file.rel),
                    local: file.size,
                    remote: remote_file.size,
                });
                queue.upload(
                    &display_name(&file.rel),
                    &file.path,
                    input.remote_dir,
                    &remote_file.rel,
                    file.size,
                    &mut warnings,
                );
            }
            Some(_) => {}
        }
    }

    if case_differences > 0 {
        warnings.push(format!(
            "{case_differences} PBO name(s) differ only by case; uploads reuse the server name"
        ));
    }

    warnings.extend(local.authority_warnings.iter().cloned());
    for (signature, authority) in input.local_signature_authorities {
        let local_ships = input.local_key_authorities.contains(authority);
        let remote_known = !input.remote_key_authorities.is_empty();
        let remote_ships = input.remote_key_authorities.contains(authority);
        if !local_ships || (remote_known && !remote_ships) {
            findings.push(Finding::KeyAuthorityMissing {
                signature: signature.clone(),
                authority: authority.clone(),
            });
        }
    }
    if !input.local_signature_authorities.is_empty() && input.remote_key_authorities.is_empty() {
        warnings.push("keys/: no remote key was read; the server key set is unknown".to_string());
    }

    Plan {
        id: local.id.clone(),
        local_dir: local.dir.clone(),
        remote_dir: input.remote_dir.to_string(),
        remote_exists: remote.exists,
        local_pbos: local.pbos().count(),
        local_signatures: local.signatures().count(),
        local_keys: local.keys().count(),
        remote_pbos: remote.addons().count(),
        remote_keys: remote.keys().count(),
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
    fn upload(
        &mut self,
        name: &str,
        local: &Path,
        pack_dir: &str,
        rel: &str,
        size: u64,
        warnings: &mut Vec<String>,
    ) {
        let remote = match inside(pack_dir, rel) {
            Ok(remote) => remote,
            Err(e) => {
                warnings.push(format!("{name}: {e}"));
                return;
            }
        };
        if self.queued.insert(remote.clone()) {
            self.actions.push(Action::Upload {
                name: name.to_string(),
                local: local.to_path_buf(),
                remote,
                size,
            });
        }
    }

    fn remove(&mut self, pack_dir: &str, rel: &str, warnings: &mut Vec<String>) {
        let remote = match inside(pack_dir, rel) {
            Ok(remote) => remote,
            Err(e) => {
                warnings.push(format!("{}: {e}", display_name(rel)));
                return;
            }
        };
        if self.queued.insert(remote.clone()) {
            self.actions.push(Action::Remove { remote });
        }
    }
}
