//! The verification phases for the pack: one batch each for the listing,
//! the `meta.cpp`/`mod.cpp` fetch and the deep download.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use super::config::Settings;
use super::parse::{self, Block};
use super::plan::{
    build_plan, join_remote, mod_dir, LocalPack, Plan, PlanInput, RemoteFile, RemotePack,
};
use super::sftp::{quote, ScratchDir, Sftp};
use crate::error::UksftaError;
use crate::util::file_sha256;

/// The inputs that stay fixed for one command run.
pub struct Verifier<'a> {
    pub client: Sftp,
    pub settings: &'a Settings,
    pub scratch: &'a ScratchDir,
    pub local: &'a LocalPack,
    pub meta_upload: PathBuf,
    pub deep: bool,
    pub require_signatures: bool,
}

impl Verifier<'_> {
    /// Run every read-only phase and build the plan.
    pub fn verify(&self) -> Result<Plan, UksftaError> {
        let dir = mod_dir(&self.settings.root, &self.local.id);
        let mut remote = self.list(&dir)?;
        self.fetch_meta(&dir, &mut remote)?;
        let deep = if self.deep {
            self.deep_hashes(&dir, &remote)?
        } else {
            HashMap::new()
        };
        Ok(build_plan(&PlanInput {
            local: self.local,
            remote_dir: &dir,
            remote: &remote,
            meta_upload: Some(&self.meta_upload),
            deep: &deep,
            require_signatures: self.require_signatures,
        }))
    }

    /// One batch lists the pack folder and its `addons/` and `keys/`.
    fn list(&self, dir: &str) -> Result<RemotePack, UksftaError> {
        let addons_dir = join_remote(dir, "addons");
        let keys_dir = join_remote(dir, "keys");
        let lines = vec![
            format!("cd {}", quote(&self.settings.root)?),
            format!("-cd {}", quote(dir)?),
            "pwd".to_string(),
            "ls -l".to_string(),
            format!("-cd {}", quote(&addons_dir)?),
            "pwd".to_string(),
            "ls -l".to_string(),
            format!("-cd {}", quote(&keys_dir)?),
            "pwd".to_string(),
            "ls -l".to_string(),
        ];
        let blocks = parse::parse_blocks(&self.client.batch(&lines)?);

        let Some(pack_block) = parse::find_block(&blocks, dir) else {
            return Ok(RemotePack::default());
        };
        for item in &pack_block.entries {
            if item.is_symlink && (item.name == "addons" || item.name == "keys") {
                return Err(UksftaError::Input(format!(
                    "{} is a symlink; refusing to follow it",
                    join_remote(dir, &item.name)
                )));
            }
        }
        let addons_block = parse::find_block(&blocks, &addons_dir);
        let keys_block = parse::find_block(&blocks, &keys_dir);
        require_listable(pack_block, "addons", &addons_dir, addons_block)?;
        require_listable(pack_block, "keys", &keys_dir, keys_block)?;

        Ok(RemotePack {
            exists: true,
            addons: files(addons_block),
            addons_symlinks: symlinks(addons_block),
            keys: files(keys_block),
            keys_symlinks: symlinks(keys_block),
            meta_id: None,
            mod_size: None,
        })
    }

    /// One batch fetches `meta.cpp` and `mod.cpp`. `-get` keeps a missing
    /// file from aborting the phase.
    fn fetch_meta(&self, dir: &str, remote: &mut RemotePack) -> Result<(), UksftaError> {
        if !remote.exists {
            return Ok(());
        }
        let meta_path = self.scratch.join("remote-meta.cpp");
        let mod_path = self.scratch.join("remote-mod.cpp");
        let _ = fs::remove_file(&meta_path);
        let _ = fs::remove_file(&mod_path);
        let lines = vec![
            format!(
                "-get {} {}",
                quote(&join_remote(dir, "meta.cpp"))?,
                quote(&meta_path.display().to_string())?
            ),
            format!(
                "-get {} {}",
                quote(&join_remote(dir, "mod.cpp"))?,
                quote(&mod_path.display().to_string())?
            ),
        ];
        self.client.batch(&lines)?;

        if let Ok(text) = fs::read_to_string(&meta_path) {
            let meta = parse::parse_meta_cpp(&text);
            remote.meta_id = meta.publishedid;
        }
        remote.mod_size = fs::metadata(&mod_path)
            .ok()
            .filter(|meta| meta.is_file())
            .map(|meta| meta.len());
        Ok(())
    }

    /// One batch downloads every remote PBO that has a local match, then
    /// hashes both copies.
    fn deep_hashes(
        &self,
        dir: &str,
        remote: &RemotePack,
    ) -> Result<HashMap<String, bool>, UksftaError> {
        let addons_dir = join_remote(dir, "addons");
        let remote_by_name: HashMap<String, &RemoteFile> = remote
            .addons
            .iter()
            .map(|file| (file.name.to_ascii_lowercase(), file))
            .collect();
        let out_dir = self.scratch.join("deep");
        fs::create_dir_all(&out_dir).map_err(UksftaError::Io)?;

        let mut lines = Vec::new();
        let mut targets = Vec::new();
        for pbo in &self.local.pbos {
            let key = pbo.name.to_ascii_lowercase();
            let Some(remote_file) = remote_by_name.get(&key) else {
                continue;
            };
            let target = out_dir.join(format!("{key}.pbo"));
            lines.push(format!(
                "-get {} {}",
                quote(&join_remote(&addons_dir, &remote_file.name))?,
                quote(&target.display().to_string())?
            ));
            targets.push((key, pbo.path.clone(), target));
        }
        if lines.is_empty() {
            return Ok(HashMap::new());
        }
        self.client.batch(&lines)?;

        let mut results = HashMap::new();
        for (key, local, downloaded) in targets {
            let same = match (file_sha256(&local), file_sha256(&downloaded)) {
                (Some(a), Some(b)) => a == b,
                _ => {
                    eprintln!("Warning: {key}: cannot hash the downloaded PBO");
                    false
                }
            };
            let _ = fs::remove_file(&downloaded);
            results.insert(key, same);
        }
        Ok(results)
    }
}

/// A directory that exists in the pack folder must also be listable.
fn require_listable(
    pack_block: &Block,
    name: &str,
    path: &str,
    block: Option<&Block>,
) -> Result<(), UksftaError> {
    let exists = pack_block
        .entries
        .iter()
        .any(|item| item.is_dir && item.name == name);
    if exists && block.is_none() {
        return Err(UksftaError::Input(format!("cannot list {path}")));
    }
    Ok(())
}

fn files(block: Option<&Block>) -> Vec<RemoteFile> {
    block.map_or_else(Vec::new, |block| {
        block
            .entries
            .iter()
            .filter(|item| !item.is_dir && !item.is_symlink)
            .map(|item| RemoteFile {
                name: item.name.clone(),
                size: item.size,
            })
            .collect()
    })
}

fn symlinks(block: Option<&Block>) -> Vec<String> {
    block.map_or_else(Vec::new, |block| {
        block
            .entries
            .iter()
            .filter(|item| item.is_symlink)
            .map(|item| item.name.clone())
            .collect()
    })
}
