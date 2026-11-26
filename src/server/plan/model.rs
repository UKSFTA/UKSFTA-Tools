//! Plan types. No filesystem access.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::lock::LockFile;
use crate::modlist::ModEntry;

/// One regular file reported by the remote listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub name: String,
    pub size: u64,
}

/// One mod folder on the server.
#[derive(Debug, Clone, Default)]
pub struct RemoteMod {
    /// False when `<root>/@<id>` is absent.
    pub exists: bool,
    /// The directory that holds the PBOs: `<root>/@<id>/addons`, or the mod
    /// folder itself when a SteamCMD copy keeps the flat shape.
    pub pbo_dir: String,
    pub files: Vec<RemoteFile>,
    pub symlinks: Vec<String>,
    /// The `publishedid` in the remote `meta.cpp`.
    pub meta_id: Option<String>,
}

/// A cached `.bisign` file and the PBO version it was made for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub path: PathBuf,
    /// Size of the signature file.
    pub size: u64,
    /// Size of the cached PBO it belongs to. None when the cache holds no
    /// copy of that PBO, so the version cannot be confirmed.
    pub pbo_size: Option<u64>,
}

/// A planned PBO upload that has no signature for the local PBO version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsignedUpload {
    pub id: String,
    pub name: String,
}

/// A per-mod outcome. `rank` orders the statuses so the worst one is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Extra,
    Unsigned,
    Stale,
    Missing,
    IdMismatch,
    NoDirectory,
    NoLock,
}

impl Status {
    pub(crate) const fn rank(self) -> u8 {
        match self {
            Status::Ok => 0,
            Status::Extra => 1,
            Status::Unsigned => 2,
            Status::Stale => 3,
            Status::Missing => 4,
            Status::IdMismatch => 5,
            Status::NoDirectory => 6,
            Status::NoLock => 7,
        }
    }

    /// A failing status. `Extra` and `Unsigned` are reported but pass.
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
            Status::IdMismatch => "id-mismatch",
            Status::NoDirectory => "no-directory",
            Status::NoLock => "no-lock",
        }
    }
}

/// One remote change the plan proposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `local: None` writes a minimal `meta.cpp` instead of copying a file.
    Upload {
        id: String,
        name: String,
        local: Option<PathBuf>,
        remote: String,
    },
    Remove {
        remote: String,
    },
}

/// One problem found on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    Missing {
        id: String,
        name: String,
    },
    StaleSize {
        id: String,
        name: String,
        local: u64,
        remote: u64,
    },
    StaleHash {
        id: String,
        name: String,
    },
    /// `remote` is the id in the remote `meta.cpp`, or None when absent.
    IdMismatch {
        id: String,
        remote: Option<String>,
    },
    Extra {
        id: String,
        name: String,
    },
    NoSignature {
        id: String,
        name: String,
    },
    NoDirectory {
        id: String,
    },
    NoLock {
        id: String,
    },
}

/// The state of one mod.
#[derive(Debug, Clone)]
pub struct ModReport {
    pub id: String,
    pub name: String,
    pub status: Status,
    pub pbos: usize,
    pub findings: Vec<Finding>,
}

/// The whole comparison.
#[derive(Debug, Clone)]
pub struct Plan {
    pub mods: Vec<ModReport>,
    pub actions: Vec<Action>,
    pub warnings: Vec<String>,
    pub unsigned_uploads: Vec<UnsignedUpload>,
    pub require_signatures: bool,
}

impl Plan {
    /// True when a mod fails, or when `--require-signatures` is set and a
    /// PBO upload has no signature for the local version.
    pub fn failed(&self) -> bool {
        self.mods.iter().any(|m| m.status.failed())
            || (self.require_signatures
                && (self.mods.iter().any(|m| m.unsigned()) || !self.unsigned_uploads.is_empty()))
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

    /// Mods that fail verification, plus unsigned mods when the caller
    /// requires signatures.
    pub fn problems(&self) -> usize {
        self.mods
            .iter()
            .filter(|m| m.status.failed() || (self.require_signatures && m.unsigned()))
            .count()
    }
}

impl ModReport {
    fn unsigned(&self) -> bool {
        self.findings
            .iter()
            .any(|f| matches!(f, Finding::NoSignature { .. }))
    }
}

/// Everything the comparison reads.
pub struct PlanInput<'a> {
    pub root: &'a str,
    pub mods: &'a [ModEntry],
    pub lock: &'a LockFile,
    pub remote: &'a HashMap<String, RemoteMod>,
    /// Lock path (for example `addons/CBA.pbo`) to local size.
    pub local_sizes: &'a HashMap<String, u64>,
    /// `<id>/<pbo>` to the cached `.bisign` files for that PBO.
    pub signatures: &'a HashMap<String, Vec<Signature>>,
    /// Workshop id to a `meta.cpp` in the local cache, copied verbatim.
    pub meta_sources: &'a HashMap<String, PathBuf>,
    /// `<id>/<pbo>` to true when the deep hashes match.
    pub deep: &'a HashMap<String, bool>,
    pub require_signatures: bool,
}
