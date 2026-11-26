//! The verification phases: one batch each for the listing, the `meta.cpp`
//! fetch and the deep download.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::config::Settings;
use super::parse;
use super::plan::{
    build_plan, join_remote, mod_dir, Plan, PlanInput, RemoteFile, RemoteMod, Signature,
};
use super::sftp::{quote, ScratchDir, Sftp};
use crate::error::UksftaError;
use crate::lock::LockFile;
use crate::modlist::ModEntry;
use crate::util::file_sha256;

/// The inputs that stay fixed for one command run.
pub struct Verifier<'a> {
    pub client: Sftp,
    pub settings: &'a Settings,
    pub scratch: &'a ScratchDir,
    pub mods: &'a [ModEntry],
    pub lock: &'a LockFile,
    pub local_sizes: HashMap<String, u64>,
    pub signatures: HashMap<String, Vec<Signature>>,
    pub meta_sources: HashMap<String, PathBuf>,
    pub deep: bool,
    pub require_signatures: bool,
}

impl Verifier<'_> {
    /// Run every read-only phase and build the plan.
    pub fn verify(&self) -> Result<Plan, UksftaError> {
        let mut remote = self.list()?;
        self.fetch_meta(&mut remote)?;
        let deep = if self.deep {
            self.deep_hashes(&remote)?
        } else {
            HashMap::new()
        };
        Ok(build_plan(&PlanInput {
            root: &self.settings.root,
            mods: self.mods,
            lock: self.lock,
            remote: &remote,
            local_sizes: &self.local_sizes,
            signatures: &self.signatures,
            meta_sources: &self.meta_sources,
            deep: &deep,
            require_signatures: self.require_signatures,
        }))
    }

    /// One batch lists the root, every mod folder and every `addons/`.
    fn list(&self) -> Result<HashMap<String, RemoteMod>, UksftaError> {
        let root = &self.settings.root;
        let mut lines = vec![
            format!("cd {}", quote(root)?),
            "pwd".to_string(),
            "ls -l".to_string(),
        ];
        for entry in self.mods {
            let dir = mod_dir(root, &entry.id);
            lines.push(format!("-cd {}", quote(&dir)?));
            lines.push("pwd".to_string());
            lines.push("ls -l".to_string());
            lines.push(format!("-cd {}", quote(&join_remote(&dir, "addons"))?));
            lines.push("pwd".to_string());
            lines.push("ls -l".to_string());
        }
        let blocks = parse::parse_blocks(&self.client.batch(&lines)?);

        let root_block = parse::find_block(&blocks, root)
            .ok_or_else(|| UksftaError::Input(format!("sftp returned no listing for {root}")))?;
        let folders = root_block
            .entries
            .iter()
            .filter(|entry| entry.is_dir && workshop_id(&entry.name).is_some())
            .count();
        if folders == 0 {
            return Err(UksftaError::Input(format!(
                "no workshop mod folders under {root}"
            )));
        }
        for entry in self.mods {
            let name = format!("@{}", entry.id);
            if let Some(item) = root_block.entries.iter().find(|item| item.name == name) {
                if item.is_symlink {
                    return Err(UksftaError::Input(format!(
                        "{name} under {root} is a symlink; refusing to follow it"
                    )));
                }
            }
        }

        let mut remote = HashMap::new();
        for entry in self.mods {
            let dir = mod_dir(root, &entry.id);
            let Some(mod_block) = parse::find_block(&blocks, &dir) else {
                remote.insert(
                    entry.id.clone(),
                    RemoteMod {
                        pbo_dir: join_remote(&dir, "addons"),
                        ..RemoteMod::default()
                    },
                );
                continue;
            };
            let addons_dir = join_remote(&dir, "addons");
            if mod_block
                .entries
                .iter()
                .any(|item| item.is_symlink && item.name == "addons")
            {
                return Err(UksftaError::Input(format!(
                    "{addons_dir} is a symlink; refusing to follow it"
                )));
            }
            let pbo_block = match parse::find_block(&blocks, &addons_dir) {
                Some(block) => block,
                None => {
                    let has_addons = mod_block
                        .entries
                        .iter()
                        .any(|item| item.is_dir && item.name == "addons");
                    if has_addons {
                        return Err(UksftaError::Input(format!("cannot list {addons_dir}")));
                    }
                    mod_block
                }
            };

            let mut files = Vec::new();
            let mut symlinks = Vec::new();
            for item in &pbo_block.entries {
                if item.is_symlink {
                    symlinks.push(item.name.clone());
                } else if !item.is_dir {
                    files.push(RemoteFile {
                        name: item.name.clone(),
                        size: item.size,
                    });
                }
            }
            remote.insert(
                entry.id.clone(),
                RemoteMod {
                    exists: true,
                    pbo_dir: pbo_block.path.trim_end_matches('/').to_string(),
                    files,
                    symlinks,
                    meta_id: None,
                },
            );
        }
        Ok(remote)
    }

    /// One batch fetches every `meta.cpp`. `-get` keeps a missing file from
    /// aborting the phase.
    fn fetch_meta(&self, remote: &mut HashMap<String, RemoteMod>) -> Result<(), UksftaError> {
        let dir = self.scratch.join("meta");
        fs::create_dir_all(&dir).map_err(UksftaError::Io)?;

        let mut ids: Vec<String> = remote
            .iter()
            .filter(|(_, item)| item.exists)
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();

        let mut lines = Vec::new();
        let mut targets = Vec::new();
        for id in ids {
            let path = dir.join(format!("{id}.cpp"));
            let remote_path = join_remote(&mod_dir(&self.settings.root, &id), "meta.cpp");
            lines.push(format!(
                "-get {} {}",
                quote(&remote_path)?,
                quote(&path.display().to_string())?
            ));
            targets.push((id, path));
        }
        if lines.is_empty() {
            return Ok(());
        }
        self.client.batch(&lines)?;

        for (id, path) in targets {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let meta: parse::MetaCpp = parse::parse_meta_cpp(&text);
            if let Some(item) = remote.get_mut(&id) {
                item.meta_id = meta.publishedid;
            }
        }
        Ok(())
    }

    /// One batch downloads every expected remote PBO, then hashes both copies.
    fn deep_hashes(
        &self,
        remote: &HashMap<String, RemoteMod>,
    ) -> Result<HashMap<String, bool>, UksftaError> {
        let dir = self.scratch.join("deep");
        fs::create_dir_all(&dir).map_err(UksftaError::Io)?;

        let mut ids: Vec<String> = remote.keys().cloned().collect();
        ids.sort();
        let mut lines = Vec::new();
        let mut targets = Vec::new();

        for id in ids {
            let (Some(item), Some(locked)) = (remote.get(&id), self.lock.mods.get(&id)) else {
                continue;
            };
            if !item.exists {
                continue;
            }
            let local_dir = dir.join(&id);
            fs::create_dir_all(&local_dir).map_err(UksftaError::Io)?;
            for file in &locked.files {
                let Some(name) = Path::new(file).file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if !name.to_ascii_lowercase().ends_with(".pbo") {
                    continue;
                }
                if !item
                    .files
                    .iter()
                    .any(|remote_file| remote_file.name == name)
                {
                    continue;
                }
                let target = local_dir.join(name);
                lines.push(format!(
                    "-get {} {}",
                    quote(&join_remote(&item.pbo_dir, name))?,
                    quote(&target.display().to_string())?
                ));
                targets.push((id.clone(), name.to_string(), PathBuf::from(file), target));
            }
        }
        if lines.is_empty() {
            return Ok(HashMap::new());
        }
        self.client.batch(&lines)?;

        let mut results = HashMap::new();
        for (id, name, local, downloaded) in targets {
            let same = match (file_sha256(&local), file_sha256(&downloaded)) {
                (Some(a), Some(b)) => a == b,
                _ => {
                    eprintln!("Warning: {id}: cannot hash {name}");
                    false
                }
            };
            let _ = fs::remove_file(&downloaded);
            results.insert(format!("{id}/{name}"), same);
        }
        Ok(results)
    }
}

/// Numeric workshop folder names are `@<id>`. Any other entry is ignored.
fn workshop_id(name: &str) -> Option<&str> {
    let id = name.strip_prefix('@')?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workshop_id_accepts_at_and_digits_only() {
        assert_eq!(workshop_id("@450814997"), Some("450814997"));
        assert_eq!(workshop_id("@"), None);
        assert_eq!(workshop_id("450814997"), None);
        assert_eq!(workshop_id("@45a"), None);
        assert_eq!(workshop_id("addons"), None);
    }
}
