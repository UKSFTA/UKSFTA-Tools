use std::collections::HashMap;
use std::path::PathBuf;

use super::compare::is_signature_of;
use super::model::UnsignedUpload;
use super::*;
use crate::lock::{LockFile, ModLockEntry};
use crate::modlist::ModEntry;

const ID: &str = "450814997";
const PBO: &str = "CBA.pbo";
const SIG: &str = "CBA.pbo.cba_3.20.0.bisign";
const ROOT: &str = "/srv/arma";

/// One mod with one signed, matching PBO on both sides.
struct Fixture {
    mods: Vec<ModEntry>,
    lock: LockFile,
    remote: HashMap<String, RemoteMod>,
    local_sizes: HashMap<String, u64>,
    signatures: HashMap<String, Vec<Signature>>,
    meta_sources: HashMap<String, PathBuf>,
    deep: HashMap<String, bool>,
}

impl Fixture {
    fn new() -> Self {
        let mods = vec![ModEntry {
            id: ID.to_string(),
            name: "CBA_A3".to_string(),
            tags: Vec::new(),
            role: "mod".to_string(),
            enabled: true,
            dependencies: Vec::new(),
        }];
        let mut lock = LockFile {
            version: 1,
            mods: HashMap::new(),
        };
        lock.mods.insert(
            ID.to_string(),
            ModLockEntry {
                files: vec![format!("addons/{PBO}")],
                name: "CBA_A3".to_string(),
                tags: Vec::new(),
                dependencies: Vec::new(),
                updated: "0".to_string(),
            },
        );
        let mut remote = HashMap::new();
        remote.insert(ID.to_string(), remote_mod(5, Some(ID)));
        let mut local_sizes = HashMap::new();
        local_sizes.insert(format!("addons/{PBO}"), 5);
        let mut signatures = HashMap::new();
        signatures.insert(
            format!("{ID}/{PBO}"),
            vec![Signature {
                path: PathBuf::from(format!("/cache/{ID}/addons/{SIG}")),
                size: 10,
                pbo_size: Some(5),
            }],
        );
        Self {
            mods,
            lock,
            remote,
            local_sizes,
            signatures,
            meta_sources: HashMap::new(),
            deep: HashMap::new(),
        }
    }

    fn plan(&self) -> Plan {
        build_plan(&PlanInput {
            root: ROOT,
            mods: &self.mods,
            lock: &self.lock,
            remote: &self.remote,
            local_sizes: &self.local_sizes,
            signatures: &self.signatures,
            meta_sources: &self.meta_sources,
            deep: &self.deep,
            require_signatures: false,
        })
    }

    fn plan_requiring_signatures(&self) -> Plan {
        build_plan(&PlanInput {
            root: ROOT,
            mods: &self.mods,
            lock: &self.lock,
            remote: &self.remote,
            local_sizes: &self.local_sizes,
            signatures: &self.signatures,
            meta_sources: &self.meta_sources,
            deep: &self.deep,
            require_signatures: true,
        })
    }

    fn remote(&mut self) -> &mut RemoteMod {
        self.remote.get_mut(ID).expect("the fixture has one mod")
    }
}

fn remote_mod(pbo_size: u64, meta_id: Option<&str>) -> RemoteMod {
    RemoteMod {
        exists: true,
        pbo_dir: format!("{ROOT}/@{ID}/addons"),
        files: vec![
            RemoteFile {
                name: PBO.to_string(),
                size: pbo_size,
            },
            RemoteFile {
                name: SIG.to_string(),
                size: 10,
            },
        ],
        symlinks: Vec::new(),
        meta_id: meta_id.map(str::to_string),
    }
}

fn upload_names(plan: &Plan) -> Vec<String> {
    plan.uploads()
        .map(|action| match action {
            Action::Upload { name, .. } => name.clone(),
            Action::Remove { .. } => String::new(),
        })
        .collect()
}

#[test]
fn matching_mod_plans_no_change() {
    let fixture = Fixture::new();
    let plan = fixture.plan();
    assert_eq!(plan.mods[0].status, Status::Ok);
    assert!(plan.mods[0].findings.is_empty());
    assert!(plan.actions.is_empty());
    assert!(!plan.failed());
}

#[test]
fn missing_pbo_is_reported_and_uploaded_with_its_signature() {
    let mut fixture = Fixture::new();
    fixture
        .remote()
        .files
        .retain(|f| f.name != PBO && f.name != SIG);

    let plan = fixture.plan();
    assert!(matches!(plan.mods[0].findings[0], Finding::Missing { .. }));
    assert_eq!(plan.mods[0].status, Status::Missing);
    assert!(plan.failed());
    assert_eq!(upload_names(&plan), vec![PBO.to_string(), SIG.to_string()]);
    assert!(plan.uploads().all(|a| match a {
        Action::Upload { remote, .. } => remote.starts_with(&format!("{ROOT}/@{ID}/addons/")),
        Action::Remove { .. } => false,
    }));
}

#[test]
fn size_mismatch_makes_the_pbo_stale() {
    let mut fixture = Fixture::new();
    fixture.remote().files[0].size = 4;

    let plan = fixture.plan();
    assert!(matches!(
        plan.mods[0].findings[0],
        Finding::StaleSize {
            local: 5,
            remote: 4,
            ..
        }
    ));
    assert_eq!(plan.mods[0].status, Status::Stale);
    assert_eq!(upload_names(&plan), vec![PBO.to_string()]);
}

#[test]
fn a_different_deep_hash_makes_the_pbo_stale() {
    let mut fixture = Fixture::new();
    fixture.deep.insert(format!("{ID}/{PBO}"), false);

    let plan = fixture.plan();
    assert!(matches!(
        plan.mods[0].findings[0],
        Finding::StaleHash { .. }
    ));
    assert_eq!(plan.mods[0].status, Status::Stale);
}

#[test]
fn an_extra_pbo_is_reported_and_removed() {
    let mut fixture = Fixture::new();
    fixture.remote().files.push(RemoteFile {
        name: "Old.pbo".to_string(),
        size: 7,
    });

    let plan = fixture.plan();
    assert!(matches!(plan.mods[0].findings[0], Finding::Extra { .. }));
    assert_eq!(plan.mods[0].status, Status::Extra);
    assert!(!plan.failed(), "an extra PBO is a warning");
    let removals: Vec<_> = plan
        .removals()
        .map(|a| match a {
            Action::Remove { remote } => remote.clone(),
            Action::Upload { .. } => String::new(),
        })
        .collect();
    assert_eq!(removals, vec![format!("{ROOT}/@{ID}/addons/Old.pbo")]);
}

#[test]
fn a_signature_file_is_never_an_extra() {
    let mut fixture = Fixture::new();
    fixture.remote().files.push(RemoteFile {
        name: "Orphan.pbo.old_1.0.bisign".to_string(),
        size: 3,
    });

    let plan = fixture.plan();
    assert!(plan.mods[0].findings.is_empty());
    assert!(plan.actions.is_empty());
}

#[test]
fn an_id_mismatch_repairs_meta_cpp_from_the_cache() {
    let mut fixture = Fixture::new();
    fixture.remote().meta_id = Some("999999999".to_string());
    fixture.meta_sources.insert(
        ID.to_string(),
        PathBuf::from(format!("/cache/{ID}/meta.cpp")),
    );

    let plan = fixture.plan();
    assert!(matches!(
        plan.mods[0].findings[0],
        Finding::IdMismatch { remote: Some(ref id), .. } if id == "999999999"
    ));
    assert_eq!(plan.mods[0].status, Status::IdMismatch);
    let action = plan.uploads().next().expect("meta.cpp is uploaded");
    assert_eq!(
        action,
        &Action::Upload {
            id: ID.to_string(),
            name: "meta.cpp".to_string(),
            local: Some(PathBuf::from(format!("/cache/{ID}/meta.cpp"))),
            remote: format!("{ROOT}/@{ID}/meta.cpp"),
        }
    );
}

#[test]
fn missing_meta_cpp_is_generated_when_the_cache_has_none() {
    let mut fixture = Fixture::new();
    fixture.remote().meta_id = None;

    let plan = fixture.plan();
    assert!(matches!(
        plan.mods[0].findings[0],
        Finding::IdMismatch { remote: None, .. }
    ));
    assert!(matches!(
        plan.uploads().next(),
        Some(Action::Upload { local: None, remote, .. }) if remote == &format!("{ROOT}/@{ID}/meta.cpp")
    ));
}

#[test]
fn a_mod_without_a_lock_entry_is_reported() {
    let mut fixture = Fixture::new();
    fixture.lock.mods.clear();

    let plan = fixture.plan();
    assert_eq!(plan.mods[0].status, Status::NoLock);
    assert!(plan.actions.is_empty());
    assert!(plan.failed());
}

#[test]
fn a_missing_mod_folder_uploads_the_files_and_meta() {
    let mut fixture = Fixture::new();
    fixture.remote.remove(ID);

    let plan = fixture.plan();
    assert!(matches!(
        plan.mods[0].findings[0],
        Finding::NoDirectory { .. }
    ));
    let names = upload_names(&plan);
    assert!(names.contains(&PBO.to_string()));
    assert!(names.contains(&"meta.cpp".to_string()));
}

#[test]
fn an_unsigned_pbo_warns_by_default_and_fails_only_when_required() {
    let mut fixture = Fixture::new();
    fixture.remote().files.retain(|f| f.name != SIG);

    let plan = fixture.plan();
    assert!(matches!(
        plan.mods[0].findings[0],
        Finding::NoSignature { .. }
    ));
    assert_eq!(plan.mods[0].status, Status::Unsigned);
    assert!(!plan.failed(), "a signature is optional by default");

    let strict = fixture.plan_requiring_signatures();
    assert!(strict.failed(), "--require-signatures makes it fail");
    assert_eq!(
        upload_names(&strict),
        vec![SIG.to_string()],
        "the cached signature is uploaded even without the flag"
    );
}

#[test]
fn a_signature_with_a_different_size_is_uploaded_again() {
    let mut fixture = Fixture::new();
    let sig = fixture
        .remote()
        .files
        .iter_mut()
        .find(|f| f.name == SIG)
        .expect("the fixture has a signature");
    sig.size = 4;

    let plan = fixture.plan();
    assert!(plan.mods[0].findings.is_empty(), "the PBO itself is fine");
    assert_eq!(upload_names(&plan), vec![SIG.to_string()]);
}

#[test]
fn too_short_or_long_signature_names_are_not_companions() {
    assert!(is_signature_of("A.pbo.key.bisign", "A.pbo"));
    assert!(!is_signature_of("AB.pbo.key.bisign", "A.pbo"));
    assert!(!is_signature_of("A.pbo", "A.pbo"));
    assert!(!is_signature_of("A.pbo.bisign", "A.pbo.key.pbo"));
}

#[test]
fn a_missing_pbo_is_uploaded_even_without_a_cached_signature() {
    let mut fixture = Fixture::new();
    fixture
        .remote()
        .files
        .retain(|f| f.name != PBO && f.name != SIG);
    fixture.signatures.clear();

    let plan = fixture.plan();
    assert_eq!(upload_names(&plan), vec![PBO.to_string()]);
    assert!(plan
        .warnings
        .iter()
        .any(|w| w.contains("no cached signature")));
    assert_eq!(
        plan.unsigned_uploads,
        vec![UnsignedUpload {
            id: ID.to_string(),
            name: PBO.to_string(),
        }]
    );
}

#[test]
fn a_signature_for_another_pbo_version_is_not_uploaded() {
    let mut fixture = Fixture::new();
    fixture
        .remote()
        .files
        .retain(|f| f.name != PBO && f.name != SIG);
    fixture
        .signatures
        .get_mut(&format!("{ID}/{PBO}"))
        .expect("the fixture has a signature")[0]
        .pbo_size = Some(6);

    let plan = fixture.plan();
    assert_eq!(
        upload_names(&plan),
        vec![PBO.to_string()],
        "the PBO uploads alone, without the stale signature"
    );
    assert!(plan
        .warnings
        .iter()
        .any(|w| w.contains("cached signature is for a different PBO version")));
    assert_eq!(plan.unsigned_uploads.len(), 1);
}

#[test]
fn require_signatures_fails_a_plan_with_an_unsigned_upload() {
    let mut fixture = Fixture::new();
    fixture
        .remote()
        .files
        .retain(|f| f.name != PBO && f.name != SIG);
    fixture.signatures.clear();

    let plan = fixture.plan_requiring_signatures();
    assert!(!plan.unsigned_uploads.is_empty());
    assert!(plan.failed(), "an unsigned upload fails under the flag");
}

#[test]
fn a_remote_symlink_is_never_written_through() {
    let mut fixture = Fixture::new();
    fixture
        .remote()
        .files
        .retain(|f| f.name != PBO && f.name != SIG);
    fixture.remote().symlinks.push(PBO.to_string());

    let plan = fixture.plan();
    assert!(plan.actions.is_empty());
    assert!(plan.warnings.iter().any(|w| w.contains("is a symlink")));
}
