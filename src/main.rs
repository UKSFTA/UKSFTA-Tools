use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

pub use std::collections::{HashMap, HashSet};

mod atomic;
mod deps;
mod docs;
mod error;
mod investigate;
mod lock;
mod modlist;
mod origin;
mod pbo;
mod prefix;
mod server;
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
pub use crate::server::ServerOptions;
pub use crate::steam::*;
pub use crate::sync::*;
pub use crate::util::*;
pub use crate::version::*;

#[derive(Parser)]
#[command(
    name = "uksfta",
    about = "UKSFTA modpack manager",
    long_about = "UKSFTA modpack manager\n\nSync, verify, inspect and export the mods of an Arma 3 modpack. Most commands run from a mod repository root that contains mod_sources.txt and addons/.",
    after_help = "Examples:\n  uksfta sync --dry-run\n  uksfta size --pbo\n  uksfta export ./modlist.html",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Sync mods from the Workshop cache into addons/
    ///
    /// Copies every mod's PBOs from the local Steam Workshop cache, writes
    /// mods.lock, and reports mods that are missing. With --modlist, writes a
    /// launcher preset for the missing mods.
    #[command(
        after_help = "Examples:\n  uksfta sync --dry-run\n  uksfta sync --offline\n  uksfta sync --modlist --resolve-deps"
    )]
    Sync {
        /// Show the planned changes without copying files
        #[arg(long)]
        dry_run: bool,
        /// Skip dependency resolution and network access
        #[arg(long)]
        offline: bool,
        /// Write a launcher preset for mods missing from the cache
        #[arg(long)]
        modlist: bool,
        /// Output path for the missing-mod preset
        #[arg(long, default_value = "missing-mods.html")]
        modlist_path: PathBuf,
        /// Resolve each mod's dependencies from Workshop pages
        #[arg(long)]
        resolve_deps: bool,
        /// Satisfy dependencies from a launcher preset HTML
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
    /// Import a launcher modlist into mod_sources.txt
    ///
    /// Adds each Steam mod from an Arma 3 launcher preset to mod_sources.txt
    /// and skips ids that are already present. Local mods have no Workshop id
    /// and are skipped with a message.
    #[command(
        after_help = "Examples:\n  uksfta import ./my-modlist.html\n  uksfta import ./my-modlist.html --dry-run"
    )]
    Import {
        /// Launcher preset HTML to read
        modlist_file: PathBuf,
        /// Print the entries without writing mod_sources.txt
        #[arg(long)]
        dry_run: bool,
    },
    /// Export mod_sources.txt as a launcher modlist
    ///
    /// Writes every enabled, non-ignored mod in mod_sources.txt as an Arma 3
    /// launcher preset. Local mods cannot be exported because they have no
    /// Workshop id.
    #[command(
        after_help = "Examples:\n  uksfta export\n  uksfta export ./pack.html --preset-name \"UKSFTA Modpack\"\n  uksfta export --dry-run"
    )]
    Export {
        /// Output file (default: modlist.html)
        #[arg(value_name = "PATH")]
        output: Option<PathBuf>,
        /// Preset name shown in the launcher (default: current directory name)
        #[arg(long, value_name = "NAME")]
        preset_name: Option<String>,
        /// Print the entries without writing a file
        #[arg(long)]
        dry_run: bool,
    },
    /// Show mod sizes from a launcher preset or the installed pack
    ///
    /// With an HTML file, reports the Workshop download size of each mod in
    /// an Arma 3 launcher preset, using the local Workshop cache.
    ///
    /// Without a file, measures the PBOs in mods.lock against addons/ and
    /// reports each mod's size, the pack total, untracked PBOs and missing
    /// files. Enabled mods in mod_sources.txt that are absent from mods.lock
    /// are listed as not installed.
    #[command(
        after_help = "Examples:\n  uksfta size\n  uksfta size --pbo --sort name\n  uksfta size ./my-modlist.html --online"
    )]
    Size {
        /// Launcher preset HTML; omit to measure the installed pack
        #[arg(value_name = "HTML")]
        modlist_file: Option<PathBuf>,
        /// List each PBO, plus untracked and missing PBO names
        #[arg(long, conflicts_with = "modlist_file")]
        pbo: bool,
        /// Order the installed report by size or name
        #[arg(
            long,
            value_enum,
            default_value = "size",
            conflicts_with = "modlist_file"
        )]
        sort: SortKey,
        /// Fetch sizes for mods missing from the local Workshop cache
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
    /// Verify the private modpack on a game server over SFTP
    ///
    /// Compares the local build with <root>/@<publishedid>/. The pack id
    /// comes from meta.cpp. Authentication is handled by sftp: key, agent
    /// or terminal prompt.
    #[command(
        after_help = "Examples:\n  uksfta server\n  uksfta server --deep\n  uksfta server --upload --dry-run\n  uksfta server --upload --prune"
    )]
    Server {
        /// Config file (default: .uksfta/server.toml when present)
        #[arg(long, value_name = "PATH")]
        config: Option<PathBuf>,
        /// user@host destination
        #[arg(long)]
        destination: Option<String>,
        /// SFTP port
        #[arg(long)]
        port: Option<u16>,
        /// Remote workshop content root
        #[arg(long, value_name = "PATH")]
        root: Option<String>,
        /// Identity file for sftp
        #[arg(long, value_name = "PATH")]
        identity: Option<PathBuf>,
        /// meta.cpp that names the pack
        #[arg(long, value_name = "PATH")]
        meta: Option<PathBuf>,
        /// Local build output (default: .hemttout/release, else .hemttout/build)
        #[arg(long, value_name = "PATH")]
        local: Option<PathBuf>,
        /// Download and hash remote PBOs (slow)
        #[arg(long)]
        deep: bool,
        /// Upload missing or stale PBOs and repair meta.cpp
        #[arg(long)]
        upload: bool,
        /// Remove remote PBOs the pack does not contain
        #[arg(long)]
        prune: bool,
        /// Also prune an extra PBO that mods.lock lists
        #[arg(long)]
        force_prune: bool,
        /// Print the plan without changing the server
        #[arg(long)]
        dry_run: bool,
        /// Fail when a PBO has no companion .bisign file
        #[arg(long)]
        require_signatures: bool,
        /// Retry an upload this many times (0 means one attempt)
        #[arg(long, default_value_t = 2)]
        retries: u8,
        /// Upload this many files in parallel (1 to 8)
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=8))]
        jobs: u8,
        /// sftp transfer buffer in bytes (4096 to 261120)
        #[arg(long, value_name = "BYTES")]
        buffer: Option<u32>,
        /// Outstanding sftp requests (1 to 256)
        #[arg(long, value_name = "N")]
        requests: Option<u16>,
        /// known_hosts file (default: ~/.ssh/known_hosts)
        #[arg(long, value_name = "PATH")]
        known_hosts: Option<PathBuf>,
        /// Host key policy: yes or accept-new (default: yes)
        #[arg(long, value_name = "POLICY")]
        host_key_policy: Option<String>,
        /// Show the sftp transfer meter (default: a terminal, single job)
        #[arg(long, group = "progress_mode")]
        progress: bool,
        /// Hide the sftp transfer meter
        #[arg(long, group = "progress_mode")]
        no_progress: bool,
        /// Local copy of the launch line to check
        #[arg(long, value_name = "FILE")]
        launch_line: Option<PathBuf>,
        /// server.cfg path on the server to check
        #[arg(long, value_name = "PATH")]
        server_cfg: Option<String>,
        /// Fail when an opt-in config check fails
        #[arg(long)]
        strict_config: bool,
    },
    /// Check that every required addon is provided or listed
    ///
    /// Compares the `CfgPatches` classes each PBO provides with the
    /// `requiredAddons` entries it declares. A requirement is provided,
    /// external or unaccounted. Advisory unless --strict.
    #[command(
        after_help = "Examples:\n  uksfta deps\n  uksfta deps --mission mission.pbo\n  uksfta deps --strict\n  uksfta deps --suggest"
    )]
    Deps {
        /// Build output to scan (default: .hemttout/release, else .hemttout/build)
        #[arg(long, value_name = "PATH")]
        local: Option<PathBuf>,
        /// Mission PBO whose addons[] list to include
        #[arg(long, value_name = "PATH")]
        mission: Option<PathBuf>,
        /// External addon list (default: .uksfta/external-addons.txt when present)
        #[arg(long, value_name = "PATH")]
        external: Option<PathBuf>,
        /// Exit 1 when any requirement is unaccounted
        #[arg(long)]
        strict: bool,
        /// Print the external-list format without writing a file
        #[arg(long)]
        suggest: bool,
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

    // Daily update notice on every command (skips the network while the
    // cached outcome is fresh). The explicit `version` command forces a
    // fresh check. UKSFTA_NO_UPDATE_CHECK disables the automatic check.
    let update_check_disabled = std::env::var_os("UKSFTA_NO_UPDATE_CHECK").is_some();
    if !update_check_disabled && !matches!(cli.command, Commands::Version) {
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
        Commands::Server {
            config,
            destination,
            port,
            root,
            identity,
            meta,
            local,
            deep,
            upload,
            prune,
            force_prune,
            dry_run,
            require_signatures,
            retries,
            jobs,
            buffer,
            requests,
            known_hosts,
            host_key_policy,
            progress,
            no_progress,
            launch_line,
            server_cfg,
            strict_config,
        } => {
            let mode = if progress {
                server::Progress::Always
            } else if no_progress {
                server::Progress::Never
            } else {
                server::Progress::Auto
            };
            server::run(ServerOptions {
                config,
                destination,
                port,
                root,
                identity,
                meta,
                local,
                deep,
                upload,
                prune,
                force_prune,
                dry_run,
                require_signatures,
                retries,
                jobs,
                buffer,
                requests,
                known_hosts,
                host_key_policy,
                progress: mode,
                launch_line,
                server_cfg,
                strict_config,
            })?
        }
        Commands::Deps {
            local,
            mission,
            external,
            strict,
            suggest,
        } => deps::run(&deps::DepsOptions {
            local: local.as_deref(),
            mission: mission.as_deref(),
            external: external.as_deref(),
            strict,
            suggest,
        })?,
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
