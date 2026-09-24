use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

pub use std::collections::{HashMap, HashSet};

mod atomic;
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
    /// Show each mod's size from a launcher modlist HTML, and the total
    Size {
        /// Path to the modlist HTML file
        modlist_file: PathBuf,
        /// Fetch sizes for mods not installed locally from the Workshop API
        #[arg(long)]
        online: bool,
    },
    /// Trace the Workshop origin of PBOs not tracked in mods.lock
    Investigate {
        /// Investigate all PBOs, including tracked ones
        #[arg(long)]
        all: bool,
        /// Check each origin against the Steam Workshop API (network)
        #[arg(long)]
        online: bool,
        /// Print the resolved identity inventory from the local cache
        /// (no network)
        #[arg(long)]
        report: bool,
    },
    /// Show the installed version and check for updates
    Version,
}

fn main() {
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
        Commands::Size {
            modlist_file,
            online,
        } => modlist_size(&modlist_file, online)?,
        Commands::Investigate {
            all,
            online,
            report,
        } => {
            if report {
                investigate_report()?;
            } else {
                investigate(all, online)?;
            }
        }
        Commands::Version => version(),
    }
    Ok(())
}
