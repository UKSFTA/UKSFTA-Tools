//! Plan types. No filesystem access.

use std::collections::HashMap;
use std::path::PathBuf;

/// One file in the remote pack folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub name: String,
    pub size: u64,
}

/// The pack folder on the server.
#[derive(Debug, Clone, Default)]
pub struct RemotePack {
    pub exists: bool,
    pub addons: Vec<RemoteFile>,
    pub keys: Vec<RemoteFile>,
    /// Names in `addons/` that are symbolic links.
    pub addons_symlinks: Vec<String>,
    /// Names in `keys/` that are symbolic links.
    pub keys_symlinks: Vec<String>,
    /// The `publishedid` in the remote `meta.cpp`.
    pub meta_id: Option<String>,
    /// Size of the remote `mod.cpp`, when present.
    pub mod_size: Option<u64>,
}

/// One file in the local build output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
}

/// The local pack: the build output plus the `meta.cpp` that names it.
#[derive(Debug, Clone, Default)]
pub struct LocalPack {
    pub id: String,
    pub name: String,
    pub dir: PathBuf,
    pub pbos: Vec<LocalFile>,
    pub signatures: Vec<LocalFile>,
    pub keys: Vec<LocalFile>,
    pub meta: Option<LocalFile>,
    pub mod_cpp: Option<LocalFile>,
    /// The `meta.cpp` that named the pack, used when the build has none.
    pub meta_source: PathBuf,
}

/// The state of the pack. `rank` orders the statuses so the worst is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Extra,
    Unsigned,
    Stale,
    Missing,
    MetaMismatch,
    NoPack,
}

impl Status {
    const fn rank(self) -> u8 {
        match self {
            Status::Ok => 0,
            Status::Extra => 1,
            Status::Unsigned => 2,
            Status::Stale => 3,
            Status::Missing => 4,
            Status::MetaMismatch => 5,
            Status::NoPack => 6,
        }
    }

    pub const fn failed(self) -> bool {
        self.rank() >= Status::Stale.rank()
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Extra => "extra",
            Status::Unsigned => "unsigned",
            Status::Stale => "stale",
            Status::Missing => "missing",
            Status::MetaMismatch => "meta-mismatch",
            Status::NoPack => "no-pack",
        }
    }
}

/// One remote change the plan proposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Upload {
        name: String,
        local: PathBuf,
        remote: String,
    },
    Remove {
        remote: String,
    },
}

/// One problem or warning-level fact about the pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    NoPack,
    /// `remote` is the id in the remote `meta.cpp`, or None when absent.
    MetaMismatch {
        remote: Option<String>,
    },
    Missing {
        name: String,
    },
    StaleSize {
        name: String,
        local: u64,
        remote: u64,
    },
    StaleHash {
        name: String,
    },
    Extra {
        name: String,
    },
    KeyMissing {
        name: String,
    },
    KeyStale {
        name: String,
        local: u64,
        remote: u64,
    },
    KeyExtra {
        name: String,
    },
    ModCppMissing,
    ModCppStale {
        local: u64,
        remote: u64,
    },
}

impl Finding {
    /// True when the finding fails the command. Extras are reported only.
    pub const fn failed(&self) -> bool {
        match self {
            Finding::Extra { .. } | Finding::KeyExtra { .. } => false,
            Finding::NoPack
            | Finding::MetaMismatch { .. }
            | Finding::Missing { .. }
            | Finding::StaleSize { .. }
            | Finding::StaleHash { .. }
            | Finding::KeyMissing { .. }
            | Finding::KeyStale { .. }
            | Finding::ModCppMissing
            | Finding::ModCppStale { .. } => true,
        }
    }

    const fn status(&self) -> Status {
        match self {
            Finding::NoPack => Status::NoPack,
            Finding::MetaMismatch { .. } => Status::MetaMismatch,
            Finding::Missing { .. } | Finding::KeyMissing { .. } | Finding::ModCppMissing => {
                Status::Missing
            }
            Finding::StaleSize { .. }
            | Finding::StaleHash { .. }
            | Finding::KeyStale { .. }
            | Finding::ModCppStale { .. } => Status::Stale,
            Finding::Extra { .. } | Finding::KeyExtra { .. } => Status::Extra,
        }
    }
}

/// The whole comparison.
#[derive(Debug, Clone)]
pub struct Plan {
    pub id: String,
    pub local_dir: PathBuf,
    pub remote_dir: String,
    pub remote_exists: bool,
    pub local_pbos: usize,
    pub local_signatures: usize,
    pub local_keys: usize,
    pub remote_pbos: usize,
    pub remote_keys: usize,
    pub findings: Vec<Finding>,
    pub actions: Vec<Action>,
    pub warnings: Vec<String>,
    /// Local PBOs that have no local signature.
    pub unsigned: usize,
    pub require_signatures: bool,
}

impl Plan {
    pub fn status(&self) -> Status {
        let mut status = Status::Ok;
        for finding in &self.findings {
            let found = finding.status();
            if found.rank() > status.rank() {
                status = found;
            }
        }
        if self.unsigned > 0 && Status::Unsigned.rank() > status.rank() {
            status = Status::Unsigned;
        }
        status
    }

    /// True when a finding fails, or when `--require-signatures` is set and
    /// a PBO has no signature.
    pub fn failed(&self) -> bool {
        self.status().failed() || (self.require_signatures && self.unsigned > 0)
    }

    pub fn problems(&self) -> usize {
        self.findings.iter().filter(|f| f.failed()).count()
    }

    pub fn local_only(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| matches!(f, Finding::Missing { .. }))
            .count()
    }

    pub fn remote_only(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| matches!(f, Finding::Extra { .. }))
            .count()
    }

    pub fn uploads(&self) -> impl Iterator<Item = &Action> {
        self.actions
            .iter()
            .filter(|a| matches!(a, Action::Upload { .. }))
    }

    pub fn removals(&self) -> impl Iterator<Item = &Action> {
        self.actions
            .iter()
            .filter(|a| matches!(a, Action::Remove { .. }))
    }
}

/// Everything the comparison reads.
pub struct PlanInput<'a> {
    pub local: &'a LocalPack,
    /// `<root>/@<id>`.
    pub remote_dir: &'a str,
    pub remote: &'a RemotePack,
    /// The file to upload when the remote `meta.cpp` is wrong.
    pub meta_upload: Option<&'a std::path::Path>,
    /// Lowercase PBO name to true when the deep hashes match.
    pub deep: &'a HashMap<String, bool>,
    pub require_signatures: bool,
}

/// True for a PBO file name.
pub fn is_pbo(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".pbo")
}

/// True for a signature file name.
pub fn is_bisign(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".bisign")
}

/// True for a key file name.
pub fn is_bikey(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".bikey")
}
