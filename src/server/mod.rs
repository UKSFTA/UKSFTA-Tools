//! Verify and repair the private modpack on a game server over SFTP.
//!
//! Only `<root>/@<publishedid>/` is read or written. Dependency tooling
//! (`mod_sources.txt`, `mods.lock`, the Workshop cache) is not used here.

mod config;
mod parse;
mod plan;
mod report;
mod sftp;
mod verify;

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::UksftaError;

use self::config::Overrides;
use self::plan::{is_bikey, is_bisign, is_pbo, Action, LocalFile, LocalPack};
use self::report::{print_plan, print_report, upload_batch};
use self::sftp::ScratchDir;
use self::sftp::Sftp;
use self::verify::Verifier;

/// The `server` command flags.
pub struct ServerOptions {
    pub config: Option<PathBuf>,
    pub destination: Option<String>,
    pub port: Option<u16>,
    pub root: Option<String>,
    pub identity: Option<PathBuf>,
    pub meta: Option<PathBuf>,
    pub local: Option<PathBuf>,
    pub deep: bool,
    pub upload: bool,
    pub prune: bool,
    pub dry_run: bool,
    pub require_signatures: bool,
}

/// Verify the pack, then upload or prune when asked.
///
/// # Errors
/// Returns `Check` when the pack fails verification, and `Input` or `Parse`
/// for a connection, usage or parse failure.
pub fn run(opts: ServerOptions) -> Result<(), UksftaError> {
    let settings = config::resolve(
        opts.config.as_deref(),
        &Overrides {
            destination: opts.destination.clone(),
            port: opts.port,
            root: opts.root.clone(),
            identity: opts.identity.clone(),
        },
    )?;

    let meta_path = opts
        .meta
        .clone()
        .unwrap_or_else(|| PathBuf::from("meta.cpp"));
    let local = load_local(&meta_path, opts.local.as_deref())?;
    let meta_upload = local
        .meta
        .as_ref()
        .map_or_else(|| local.meta_source.clone(), |file| file.path.clone());
    let scratch = ScratchDir::new("uksfta-server-")?;
    let verifier = Verifier {
        client: Sftp::new(&settings),
        settings: &settings,
        scratch: &scratch,
        local: &local,
        meta_upload,
        deep: opts.deep,
        require_signatures: opts.require_signatures,
    };

    println!("pack {} {}", local.id, local.name);
    println!(
        "server: {}:{} root {}",
        settings.destination, settings.port, settings.root
    );

    let mut plan = verifier.verify()?;
    print_report(&plan);
    print_plan(&plan);

    if opts.dry_run {
        println!("dry run: no changes made");
    } else if opts.upload {
        let batch = upload_batch(&plan)?;
        if !batch.is_empty() {
            verifier.client.batch(&batch)?;
            let puts = batch.iter().filter(|line| line.starts_with("put ")).count();
            println!("upload: {puts} file(s) written");
            plan = verifier.verify()?;
            println!("after upload:");
            print_report(&plan);
        }
    }

    if opts.prune && !opts.dry_run {
        let removals: Vec<String> = plan.removals().map(remove_path).collect();
        if !removals.is_empty() {
            let mut batch = Vec::new();
            for remote in &removals {
                batch.push(format!("rm {}", self::sftp::quote(remote)?));
            }
            verifier.client.batch(&batch)?;
            for remote in &removals {
                println!("removed {remote}");
            }
        }
    }

    if plan.failed() {
        let mut reasons = Vec::new();
        let problems = plan.problems();
        if problems > 0 {
            reasons.push(format!("{problems} pack problem(s)"));
        }
        if opts.require_signatures && plan.unsigned > 0 {
            reasons.push(format!(
                "{} of {} PBO(s) have no signature",
                plan.unsigned, plan.local_pbos
            ));
        }
        return Err(UksftaError::Check(reasons.join(", ")));
    }
    Ok(())
}

fn remove_path(action: &Action) -> String {
    match action {
        Action::Remove { remote } | Action::Upload { remote, .. } => remote.clone(),
    }
}

/// Read the pack identity and the local build output.
///
/// # Errors
/// Returns `Input` when `meta.cpp` or the build output is missing, and
/// `Parse` when `meta.cpp` has no numeric `publishedid`.
fn load_local(meta_path: &Path, local: Option<&Path>) -> Result<LocalPack, UksftaError> {
    let text = fs::read_to_string(meta_path).map_err(|_| {
        UksftaError::Input(format!(
            "no meta.cpp at {}\nRun from the pack repo root, or pass --meta PATH",
            meta_path.display()
        ))
    })?;
    let meta = parse::parse_meta_cpp(&text);
    let Some(id) = meta.publishedid else {
        return Err(UksftaError::Parse(format!(
            "{} has no publishedid",
            meta_path.display()
        )));
    };
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(UksftaError::Parse(format!(
            "publishedid in {} is not numeric: {id}",
            meta_path.display()
        )));
    }
    let name = meta.name.unwrap_or_else(|| format!("Pack {id}"));

    let dir = local.map_or_else(default_local_dir, Path::to_path_buf);
    let addons = dir.join("addons");
    if !addons.is_dir() {
        return Err(UksftaError::Input(format!(
            "no addons directory in {}\nBuild the pack first, or pass --local PATH",
            dir.display()
        )));
    }
    let pbos = collect(&addons, is_pbo);
    if pbos.is_empty() {
        return Err(UksftaError::Input(format!(
            "no PBOs in {}",
            addons.display()
        )));
    }
    let signatures = collect(&addons, is_bisign);
    let keys = collect(&dir.join("keys"), is_bikey);
    let meta_file = local_file(&dir.join("meta.cpp"));
    let mod_cpp = local_file(&dir.join("mod.cpp"));

    Ok(LocalPack {
        id,
        name,
        dir,
        pbos,
        signatures,
        keys,
        meta: meta_file,
        mod_cpp,
        meta_source: meta_path.to_path_buf(),
    })
}

/// `.hemttout/release` when it exists, else `.hemttout/build`.
fn default_local_dir() -> PathBuf {
    let release = PathBuf::from(".hemttout/release");
    if release.is_dir() {
        release
    } else {
        PathBuf::from(".hemttout/build")
    }
}

/// The regular files in `dir` whose name passes `keep`, sorted by name.
fn collect(dir: &Path, keep: fn(&str) -> bool) -> Vec<LocalFile> {
    let Ok(items) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<LocalFile> = items
        .flatten()
        .filter_map(|item| {
            let path = item.path();
            let name = path.file_name()?.to_str()?;
            if !keep(name) {
                return None;
            }
            let meta = item.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some(LocalFile {
                name: name.to_string(),
                path,
                size: meta.len(),
            })
        })
        .collect();
    files.sort_by(|a, b| a.name.cmp(&b.name));
    files
}

fn local_file(path: &Path) -> Option<LocalFile> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(LocalFile {
        name: path.file_name()?.to_str()?.to_string(),
        path: path.to_path_buf(),
        size: meta.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const META: &str =
        "protocol = 1;\npublishedid = 3662499431;\nname = \"UKSF Taskforce Alpha Maps\";\n";

    fn pack_dir(root: &Path) -> PathBuf {
        let dir = root.join(".hemttout/release");
        fs::create_dir_all(dir.join("addons")).unwrap();
        fs::create_dir_all(dir.join("keys")).unwrap();
        fs::write(dir.join("addons/uksfta_main.pbo"), b"pbo").unwrap();
        fs::write(dir.join("addons/uksfta_main.pbo.key.bisign"), b"sig").unwrap();
        fs::write(dir.join("keys/uksfta.bikey"), b"key").unwrap();
        fs::write(dir.join("meta.cpp"), META).unwrap();
        fs::write(dir.join("mod.cpp"), b"mod").unwrap();
        dir
    }

    #[test]
    fn load_local_reads_the_pack_identity_and_build_output() {
        let root = tempfile::TempDir::new().unwrap();
        let dir = pack_dir(root.path());
        let meta = root.path().join(".hemttout/release/meta.cpp");

        let local = load_local(&meta, Some(&dir)).unwrap();
        assert_eq!(local.id, "3662499431");
        assert_eq!(local.name, "UKSF Taskforce Alpha Maps");
        assert_eq!(local.pbos.len(), 1);
        assert_eq!(local.signatures.len(), 1);
        assert_eq!(local.keys.len(), 1);
        assert!(local.meta.is_some());
        assert!(local.mod_cpp.is_some());
    }

    #[test]
    fn load_local_rejects_a_missing_meta_cpp() {
        let root = tempfile::TempDir::new().unwrap();
        let err = load_local(&root.path().join("meta.cpp"), None).unwrap_err();
        assert!(matches!(err, UksftaError::Input(_)));
    }

    #[test]
    fn load_local_rejects_a_non_numeric_id() {
        let root = tempfile::TempDir::new().unwrap();
        let dir = pack_dir(root.path());
        let meta = root.path().join("meta.cpp");
        fs::write(&meta, "publishedid = \"pack\";\n").unwrap();
        let err = load_local(&meta, Some(&dir)).unwrap_err();
        assert!(matches!(err, UksftaError::Parse(_)));
    }

    #[test]
    fn load_local_requires_an_addons_directory_with_pbos() {
        let root = tempfile::TempDir::new().unwrap();
        let dir = root.path().join("build");
        fs::create_dir_all(&dir).unwrap();
        let meta = root.path().join("meta.cpp");
        fs::write(&meta, META).unwrap();

        let err = load_local(&meta, Some(&dir)).unwrap_err();
        assert!(matches!(err, UksftaError::Input(_)));

        fs::create_dir_all(dir.join("addons")).unwrap();
        let err = load_local(&meta, Some(&dir)).unwrap_err();
        assert!(matches!(err, UksftaError::Input(_)));
    }
}
