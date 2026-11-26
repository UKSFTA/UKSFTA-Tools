//! Verify and repair a game server's workshop tree over SFTP.
//!
//! Every remote operation uses the `sftp` client in batch mode. No other
//! remote tool is started, and no credential is read, stored or printed.

mod cache;
mod config;
mod parse;
mod plan;
mod report;
mod sftp;
mod verify;

use std::path::{Path, PathBuf};

use crate::error::UksftaError;
use crate::lock::load_lock;
use crate::modlist::parse_mod_sources;

use self::config::Overrides;
use self::report::{print_plan, print_report, upload_batch};
use self::sftp::{quote, ScratchDir, Sftp};
use self::verify::Verifier;

/// The `server` command flags.
pub struct ServerOptions {
    pub config: Option<PathBuf>,
    pub destination: Option<String>,
    pub port: Option<u16>,
    pub root: Option<String>,
    pub identity: Option<PathBuf>,
    pub deep: bool,
    pub upload: bool,
    pub prune: bool,
    pub dry_run: bool,
    pub require_signatures: bool,
}

/// Verify the server, then upload or prune when asked.
///
/// # Errors
/// Returns `Check` when a mod fails verification, and `Input` or `Parse`
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

    let (mods, _ignored) = parse_mod_sources(Path::new("mod_sources.txt"))?;
    if mods.is_empty() {
        return Err(UksftaError::Input(
            "no enabled mods in mod_sources.txt".to_string(),
        ));
    }
    let lock = load_lock(Path::new("mods.lock"))?;
    cache::warn_missing_local(&mods, &lock);

    let caches = cache::workshop_caches();
    let scratch = ScratchDir::new("uksfta-server-")?;
    let verifier = Verifier {
        client: Sftp::new(&settings),
        settings: &settings,
        scratch: &scratch,
        mods: &mods,
        lock: &lock,
        local_sizes: cache::local_sizes(&lock),
        signatures: cache::signatures(&caches, &mods),
        meta_sources: cache::meta_sources(&caches, &mods),
        deep: opts.deep,
        require_signatures: opts.require_signatures,
    };

    println!(
        "server: {}:{} root {}",
        settings.destination, settings.port, settings.root
    );
    println!("checking {} mod(s) over sftp", mods.len());

    let mut plan = verifier.verify()?;
    print_report(&plan);
    print_plan(&plan);
    let unsigned_uploads = plan.unsigned_uploads.clone();

    if opts.dry_run {
        println!("dry run: no changes made");
    } else if opts.upload {
        let batch = upload_batch(&plan, verifier.scratch)?;
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
                batch.push(format!("rm {}", quote(remote)?));
            }
            verifier.client.batch(&batch)?;
            for remote in &removals {
                println!("removed {remote}");
            }
        }
    }

    if plan.failed() {
        return Err(UksftaError::Check(format!(
            "{} server problem(s) found",
            plan.problems()
        )));
    }
    if opts.require_signatures && !unsigned_uploads.is_empty() {
        return Err(UksftaError::Check(format!(
            "{} uploaded PBO(s) have no matching signature",
            unsigned_uploads.len()
        )));
    }
    Ok(())
}

fn remove_path(action: &plan::Action) -> String {
    match action {
        plan::Action::Remove { remote } | plan::Action::Upload { remote, .. } => remote.clone(),
    }
}
