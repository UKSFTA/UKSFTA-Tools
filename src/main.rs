use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

pub use std::collections::{HashMap, HashSet};

mod atomic;
mod docs;
mod error;
mod investigate;
mod lock;
mod modlist;
mod origin;
mod pbo;
mod prefix;
mod steam;
mod sync;
mod util;
mod version;
mod workshop_api;

use crate::error::UksftaError;

pub use crate::docs::*;
pub use crate::investigate::*;
pub use crate::lock::*;
pub use crate::modlist::*;
pub use crate::origin::*;
pub use crate::pbo::*;
pub use crate::steam::*;
pub use crate::sync::*;
pub use crate::util::*;
pub use crate::version::*;

#[derive(Parser)]
#[command(name = "uksfta", about = "UKSFTA modpack manager", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Sync mods from Workshop cache into addons/
    Sync {
        /// Show what would change without doing it
        #[arg(long)]
        dry_run: bool,
        /// Skip dependency resolution (offline mode)
        #[arg(long)]
        offline: bool,
        /// Generate an Arma 3 launcher modlist for missing mods
        #[arg(long)]
        modlist: bool,
        /// Output path for the modlist HTML (default: missing-mods.html)
        #[arg(long, default_value = "missing-mods.html")]
        modlist_path: PathBuf,
        /// Resolve every mod's dependencies from Workshop pages
        #[arg(long)]
        resolve_deps: bool,
        /// Satisfy discovered dependencies from an Arma 3 launcher modlist HTML file
        #[arg(long, value_name = "PATH")]
        fill_from: Option<PathBuf>,
    },
    /// Show which PBOs came from which Workshop mod
    Identify,
    /// Verify all PBOs are present
    Verify,
    /// Audit each mod's PBOs against addons/ (present or missing)
    Audit {
        /// Show only missing PBOs and per-mod counts
        #[arg(long)]
        missing_only: bool,
    },
    /// Check for updates available in Workshop cache
    Updates,
    /// Import mods from an Arma 3 launcher modlist HTML file
    Import {
        /// Path to the modlist HTML file
        modlist_file: PathBuf,
        /// Show what would be imported without modifying mod_sources.txt
        #[arg(long)]
        dry_run: bool,
    },
    /// Export mod_sources.txt as an Arma 3 launcher modlist
    Export {
        /// Output path for the modlist HTML (default: modlist.html)
        #[arg(value_name = "PATH")]
        output: Option<PathBuf>,
        /// Preset name shown in the launcher (default: current directory name)
        #[arg(long, value_name = "NAME")]
        preset_name: Option<String>,
        /// Print the entries without writing the file
        #[arg(long)]
        dry_run: bool,
    },
    /// Show mod sizes from a modlist HTML file, or the installed pack size
    Size {
        /// Path to the modlist HTML file
        modlist_file: Option<PathBuf>,
        /// Print one line per PBO under each mod
        #[arg(long, conflicts_with = "modlist_file")]
        pbo: bool,
        /// Sort by size (default) or by name
        #[arg(
            long,
            value_enum,
            default_value = "size",
            conflicts_with = "modlist_file"
        )]
        sort: SortKey,
        /// Fetch sizes for mods not installed locally from the Workshop API
        #[arg(long, requires = "modlist_file")]
        online: bool,
    },
    /// Trace the Workshop origin of PBOs not tracked in mods.lock
    Investigate {
        /// Investigate all PBOs, including tracked ones
        #[arg(long)]
        all: bool,
        /// Check origins and resolve identities against the Workshop (network)
        #[arg(long)]
        online: bool,
        /// Print the resolution report; with --online, resolve first
        #[arg(long)]
        report: bool,
    },
    /// Show the installed version and check for updates
    Version,
    /// Update the description and changelog files named in [hemtt.publish]
    Docs {
        /// Scan this directory for HEMTT projects instead of the current one
        #[arg(long, value_name = "PATH")]
        root: Option<PathBuf>,
        /// Show the generated text without changing files
        #[arg(long, conflicts_with = "check")]
        dry_run: bool,
        /// Fail when a configured file is missing or out of date
        #[arg(long)]
        check: bool,
        /// Look up missing mod names from the Steam Workshop API
        #[arg(long)]
        online: bool,
        /// Refetch cached Workshop names instead of using the cache
        #[arg(long, requires = "online")]
        refresh_names: bool,
        /// Replace files and sections that uksfta did not generate
        #[arg(long)]
        force: bool,
    },
}

/// Restore the default `SIGPIPE` handler so a closed pipe ends the process
/// quietly, as a command-line reader expects.
#[cfg(unix)]
fn reset_sigpipe() {
    // SAFETY: runs before any thread is spawned and only restores the default
    // handler, so no Rust code observes the signal state change.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

fn main() {
    #[cfg(unix)]
    reset_sigpipe();

    let cli = Cli::parse();

    // Daily update notice on every command (skips network if checked
    // within 24h). The explicit `version` command forces a fresh check.
    if !matches!(cli.command, Commands::Version) {
        check_for_updates(false);
    }

    if let Err(e) = run(cli) {
        eprintln!("Error: {e}");
        std::process::exit(e.exit_code());
    }
}

/// Dispatch one command. `main` owns the process exit code.
fn run(cli: Cli) -> Result<(), UksftaError> {
    match cli.command {
        Commands::Sync {
            dry_run,
            offline,
            modlist,
            modlist_path,
            resolve_deps,
            fill_from,
        } => {
            let sources_path = Path::new("mod_sources.txt");
            migrate_legacy_if_needed(sources_path, dry_run)?;
            let (mods, ignored) = parse_mod_sources(sources_path)?;
            if mods.is_empty() {
                return Err(UksftaError::Input(
                    "No mods found in mod_sources.txt".to_string(),
                ));
            }
            println!("Found {} mods, {} ignored", mods.len(), ignored.len());
            if offline && resolve_deps {
                eprintln!("Warning: --offline ignores --resolve-deps.");
            }
            let opts = SyncOptions {
                sources_path,
                dry_run,
                offline,
                modlist,
                modlist_path: &modlist_path,
                resolve_deps,
                fill_from: fill_from.as_deref(),
            };
            sync_mods(&mods, &ignored, &opts)?;
        }
        Commands::Identify => identify()?,
        Commands::Verify => verify()?,
        Commands::Audit { missing_only } => audit(missing_only)?,
        Commands::Updates => check_updates()?,
        Commands::Import {
            modlist_file,
            dry_run,
        } => import_modlist(&modlist_file, dry_run)?,
        Commands::Export {
            output,
            preset_name,
            dry_run,
        } => export_modlist(
            Path::new("mod_sources.txt"),
            Path::new("mods.lock"),
            output.as_deref(),
            preset_name.as_deref(),
            dry_run,
        )?,
        Commands::Size {
            modlist_file,
            pbo,
            sort,
            online,
        } => match modlist_file {
            Some(path) => modlist_size(&path, online)?,
            None => installed_size(pbo, sort)?,
        },
        Commands::Investigate {
            all,
            online,
            report,
        } => investigate(all, online, report)?,
        Commands::Version => version(),
        Commands::Docs {
            root,
            dry_run,
            check,
            online,
            refresh_names,
            force,
        } => docs(&DocsOptions {
            root: root.as_deref(),
            dry_run,
            check,
            online,
            refresh_names,
            force,
        })?,
    }
    Ok(())
}
