//! Build the plan from the local pack and the remote listing.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::model::{
    Action, Finding, ModReport, Plan, PlanInput, RemoteFile, Signature, Status, UnsignedUpload,
};
use super::{join_remote, mod_dir};

/// True for a name that is a companion of `pbo`, for example
/// `ace_zeus.pbo.ace_3.21.3.bisign`.
pub(super) fn is_signature_of(name: &str, pbo: &str) -> bool {
    name.strip_prefix(pbo)
        .is_some_and(|rest| rest.starts_with('.') && rest.ends_with(".bisign"))
}

fn is_signature(name: &str) -> bool {
    name.ends_with(".bisign")
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name()?.to_str().map(str::to_string)
}

/// Build the plan.
pub fn build_plan(input: &PlanInput<'_>) -> Plan {
    let mut mods = Vec::new();
    let mut queue = Queue::default();
    let mut warnings = Vec::new();
    let mut unsigned_uploads = Vec::new();

    for entry in input.mods {
        let mut findings = Vec::new();
        let Some(locked) = input.lock.mods.get(&entry.id) else {
            findings.push(Finding::NoLock {
                id: entry.id.clone(),
            });
            mods.push(ModReport {
                id: entry.id.clone(),
                name: entry.name.clone(),
                status: Status::NoLock,
                pbos: 0,
                findings,
            });
            continue;
        };

        let name = if locked.name.is_empty() {
            entry.name.clone()
        } else {
            locked.name.clone()
        };
        let remote = input.remote.get(&entry.id).filter(|r| r.exists);
        let dir = mod_dir(input.root, &entry.id);
        let pbo_dir = remote.map_or_else(|| join_remote(&dir, "addons"), |r| r.pbo_dir.clone());

        let local: Vec<(String, PathBuf, u64)> = locked
            .files
            .iter()
            .filter_map(|path| match file_name(Path::new(path)) {
                Some(name) => input
                    .local_sizes
                    .get(path)
                    .map(|size| (name, PathBuf::from(path), *size)),
                None => None,
            })
            .collect();
        let local_names: HashSet<&str> = local.iter().map(|(n, _, _)| n.as_str()).collect();

        if remote.is_none() {
            findings.push(Finding::NoDirectory {
                id: entry.id.clone(),
            });
        }
        let remote_by_name: HashMap<&str, &RemoteFile> = remote.map_or_else(HashMap::new, |r| {
            r.files.iter().map(|f| (f.name.as_str(), f)).collect()
        });

        for (pbo, local_path, local_size) in &local {
            let key = format!("{}/{}", entry.id, pbo);
            let found = remote_by_name.get(pbo.as_str()).copied();
            let blocked = remote.is_some_and(|r| r.symlinks.iter().any(|s| s == pbo));
            let mut stale = false;

            match found {
                None => {
                    findings.push(Finding::Missing {
                        id: entry.id.clone(),
                        name: pbo.clone(),
                    });
                    stale = true;
                }
                Some(file) if file.size != *local_size => {
                    findings.push(Finding::StaleSize {
                        id: entry.id.clone(),
                        name: pbo.clone(),
                        local: *local_size,
                        remote: file.size,
                    });
                    stale = true;
                }
                Some(_) if input.deep.get(&key) == Some(&false) => {
                    findings.push(Finding::StaleHash {
                        id: entry.id.clone(),
                        name: pbo.clone(),
                    });
                    stale = true;
                }
                Some(_) => {}
            }

            let signed = remote_by_name
                .keys()
                .any(|remote_name| is_signature_of(remote_name, pbo));
            if remote.is_some() && found.is_some() && !signed {
                findings.push(Finding::NoSignature {
                    id: entry.id.clone(),
                    name: pbo.clone(),
                });
            }

            if blocked {
                warnings.push(format!(
                    "{}: {pbo} is a symlink on the server; not written",
                    entry.id
                ));
                continue;
            }

            let signatures = input.signatures.get(&key).map_or(&[][..], Vec::as_slice);
            let valid: Vec<&Signature> = signatures
                .iter()
                .filter(|signature| signature.pbo_size == Some(*local_size))
                .collect();

            if stale {
                queue.upload(&entry.id, pbo, local_path, format!("{pbo_dir}/{pbo}"));
                if valid.is_empty() {
                    if signatures.is_empty() {
                        warnings.push(format!("{}: {pbo}: no cached signature", entry.id));
                    } else {
                        warnings.push(format!(
                            "{}: {pbo}: cached signature is for a different PBO version and was not uploaded",
                            entry.id
                        ));
                    }
                    unsigned_uploads.push(UnsignedUpload {
                        id: entry.id.clone(),
                        name: pbo.clone(),
                    });
                }
            }
            for signature in valid {
                let Some(sig_name) = file_name(&signature.path) else {
                    continue;
                };
                let out_of_date = remote_by_name
                    .get(sig_name.as_str())
                    .is_none_or(|file| file.size != signature.size);
                if out_of_date {
                    queue.upload(
                        &entry.id,
                        &sig_name,
                        &signature.path,
                        format!("{pbo_dir}/{sig_name}"),
                    );
                }
            }
        }

        // The meta.cpp id must name this workshop item.
        let meta_matches = remote
            .and_then(|r| r.meta_id.as_deref())
            .is_some_and(|id| id == entry.id);
        let can_create = remote.is_some() || queue.has_upload(&entry.id);
        if !meta_matches && can_create {
            if remote.is_some() {
                findings.push(Finding::IdMismatch {
                    id: entry.id.clone(),
                    remote: remote.and_then(|r| r.meta_id.clone()),
                });
            }
            let local = input.meta_sources.get(&entry.id).cloned();
            queue.upload_meta(&entry.id, local, &dir);
        }

        if let Some(remote) = remote {
            for file in &remote.files {
                if local_names.contains(file.name.as_str())
                    || is_signature(&file.name)
                    || !file.name.to_ascii_lowercase().ends_with(".pbo")
                {
                    continue;
                }
                findings.push(Finding::Extra {
                    id: entry.id.clone(),
                    name: file.name.clone(),
                });
                queue.remove(format!("{pbo_dir}/{}", file.name));
            }
        }

        mods.push(ModReport {
            id: entry.id.clone(),
            name,
            status: status_of(&findings),
            pbos: local.len(),
            findings,
        });
    }

    Plan {
        mods,
        actions: queue.actions,
        warnings,
        unsigned_uploads,
        require_signatures: input.require_signatures,
    }
}

fn status_of(findings: &[Finding]) -> Status {
    let mut status = Status::Ok;
    for finding in findings {
        let found = match finding {
            Finding::Missing { .. } => Status::Missing,
            Finding::StaleSize { .. } | Finding::StaleHash { .. } => Status::Stale,
            Finding::IdMismatch { .. } => Status::IdMismatch,
            Finding::Extra { .. } => Status::Extra,
            Finding::NoSignature { .. } => Status::Unsigned,
            Finding::NoDirectory { .. } => Status::NoDirectory,
            Finding::NoLock { .. } => Status::NoLock,
        };
        if found.rank() > status.rank() {
            status = found;
        }
    }
    status
}

/// Collect actions once per remote path.
#[derive(Default)]
struct Queue {
    actions: Vec<Action>,
    queued: HashSet<String>,
}

impl Queue {
    fn upload(&mut self, id: &str, name: &str, local: &Path, remote: String) {
        if self.queued.insert(remote.clone()) {
            self.actions.push(Action::Upload {
                id: id.to_string(),
                name: name.to_string(),
                local: Some(local.to_path_buf()),
                remote,
            });
        }
    }

    fn upload_meta(&mut self, id: &str, local: Option<PathBuf>, dir: &str) {
        let remote = join_remote(dir, "meta.cpp");
        if self.queued.insert(remote.clone()) {
            self.actions.push(Action::Upload {
                id: id.to_string(),
                name: "meta.cpp".to_string(),
                local,
                remote,
            });
        }
    }

    fn has_upload(&self, id: &str) -> bool {
        self.actions
            .iter()
            .any(|a| matches!(a, Action::Upload { id: other, .. } if other == id))
    }

    fn remove(&mut self, remote: String) {
        if self.queued.insert(remote.clone()) {
            self.actions.push(Action::Remove { remote });
        }
    }
}
