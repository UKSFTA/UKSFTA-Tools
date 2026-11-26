use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::compare::{is_signature_of, signature_owner};
use super::model::Status;
use super::*;

const ID: &str = "3662499431";
const NAME: &str = "UKSF Taskforce Alpha Maps";
const PBO: &str = "uksfta_main.pbo";
const SIG: &str = "uksfta_main.pbo.key.bisign";
const KEY: &str = "uksfta.bikey";
const DIR: &str = "/srv/arma/@3662499431";

fn local_file(dir: &str, name: &str, size: u64) -> LocalFile {
    LocalFile {
        name: name.to_string(),
        path: PathBuf::from(format!("{dir}/{name}")),
        size,
    }
}

fn remote_file(name: &str, size: u64) -> RemoteFile {
    RemoteFile {
        name: name.to_string(),
        size,
    }
}

/// A pack whose local build and remote folder match exactly.
struct Fixture {
    local: LocalPack,
    remote: RemotePack,
    deep: HashMap<String, bool>,
    locked: HashSet<String>,
    force_prune: bool,
}

impl Fixture {
    fn new() -> Self {
        let local = LocalPack {
            id: ID.to_string(),
            name: NAME.to_string(),
            dir: PathBuf::from("/local"),
            pbos: vec![local_file("/local/addons", PBO, 100)],
            signatures: vec![local_file("/local/addons", SIG, 10)],
            keys: vec![local_file("/local/keys", KEY, 50)],
            meta: Some(local_file("/local", "meta.cpp", 4)),
            mod_cpp: Some(local_file("/local", "mod.cpp", 20)),
            meta_source: PathBuf::from("/repo/meta.cpp"),
        };
        let remote = RemotePack {
            exists: true,
            addons: vec![remote_file(PBO, 100), remote_file(SIG, 10)],
            keys: vec![remote_file(KEY, 50)],
            addons_symlinks: Vec::new(),
            keys_symlinks: Vec::new(),
            meta_id: Some(ID.to_string()),
            mod_size: Some(20),
        };
        Self {
            local,
            remote,
            deep: HashMap::new(),
            locked: HashSet::new(),
            force_prune: false,
        }
    }

    fn plan(&self) -> Plan {
        self.build(false)
    }

    fn build(&self, require_signatures: bool) -> Plan {
        build_plan(&PlanInput {
            local: &self.local,
            remote_dir: DIR,
            remote: &self.remote,
            meta_upload: Some(Path::new("/local/meta.cpp")),
            deep: &self.deep,
            locked: &self.locked,
            force_prune: self.force_prune,
            require_signatures,
        })
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

fn remote_targets(plan: &Plan) -> Vec<String> {
    plan.actions
        .iter()
        .map(|action| match action {
            Action::Upload { remote, .. } | Action::Remove { remote } => remote.clone(),
        })
        .collect()
}

#[test]
fn a_matching_pack_plans_no_change() {
    let plan = Fixture::new().plan();
    assert_eq!(plan.status(), Status::Ok);
    assert!(plan.findings.is_empty(), "{:?}", plan.findings);
    assert!(plan.actions.is_empty());
    assert!(!plan.failed());
    assert_eq!(plan.id, ID);
    assert_eq!(plan.local_pbos, 1);
    assert_eq!(plan.remote_pbos, 1);
}

#[test]
fn a_missing_pbo_is_reported_and_uploaded_with_its_signature() {
    let mut fixture = Fixture::new();
    fixture.remote.addons.clear();

    let plan = fixture.plan();
    assert_eq!(
        plan.findings[0],
        Finding::Missing {
            name: PBO.to_string()
        }
    );
    assert_eq!(plan.status(), Status::Missing);
    assert!(plan.failed());
    assert_eq!(plan.local_only(), 1);
    assert_eq!(
        remote_targets(&plan),
        vec![format!("{DIR}/addons/{PBO}"), format!("{DIR}/addons/{SIG}")]
    );
}

#[test]
fn a_case_only_difference_is_the_same_file() {
    let mut fixture = Fixture::new();
    fixture.remote.addons[0].name = "UKSFTA_MAIN.PBO".to_string();

    let plan = fixture.plan();
    assert_eq!(plan.status(), Status::Ok);
    assert!(plan.actions.is_empty(), "no duplicate is created");
    assert_eq!(plan.warnings.len(), 1, "{:?}", plan.warnings);
    assert!(plan.warnings[0].contains("1 PBO name(s) differ only by case"));
}

#[test]
fn case_only_differences_aggregate_into_one_warning() {
    let mut fixture = Fixture::new();
    fixture
        .local
        .pbos
        .push(local_file("/local/addons", "uksfta_extra.pbo", 5));
    fixture
        .remote
        .addons
        .push(remote_file("UKSFTA_EXTRA.PBO", 5));

    let plan = fixture.plan();
    assert!(plan.findings.is_empty());
    assert_eq!(plan.warnings.len(), 1, "{:?}", plan.warnings);
    assert!(plan.warnings[0].contains("1 PBO name(s) differ only by case"));
}

#[test]
fn a_case_only_difference_uploads_to_the_server_name() {
    let mut fixture = Fixture::new();
    fixture.remote.addons[0].name = "UKSFTA_MAIN.PBO".to_string();
    fixture.remote.addons[0].size = 90;

    let plan = fixture.plan();
    let targets = remote_targets(&plan);
    assert_eq!(
        targets[0],
        format!("{DIR}/addons/UKSFTA_MAIN.PBO"),
        "the existing server name is used"
    );
    assert!(
        !targets.iter().any(|t| t.ends_with(&format!("/{PBO}"))),
        "no lowercase duplicate is created"
    );
}

#[test]
fn a_size_mismatch_makes_the_pbo_stale() {
    let mut fixture = Fixture::new();
    fixture.remote.addons[0].size = 90;

    let plan = fixture.plan();
    assert_eq!(
        plan.findings[0],
        Finding::StaleSize {
            name: PBO.to_string(),
            local: 100,
            remote: 90
        }
    );
    assert_eq!(plan.status(), Status::Stale);
    assert_eq!(upload_names(&plan), vec![PBO.to_string(), SIG.to_string()]);
}

#[test]
fn a_deep_hash_mismatch_makes_the_pbo_stale() {
    let mut fixture = Fixture::new();
    fixture.deep.insert(PBO.to_string(), false);

    let plan = fixture.plan();
    assert!(matches!(plan.findings[0], Finding::StaleHash { .. }));
    assert_eq!(plan.status(), Status::Stale);
}

#[test]
fn an_extra_remote_pbo_is_reported_and_removed() {
    let mut fixture = Fixture::new();
    fixture.remote.addons.push(remote_file("Old.pbo", 7));

    let plan = fixture.plan();
    assert_eq!(
        plan.findings[0],
        Finding::Extra {
            name: "Old.pbo".to_string()
        }
    );
    assert_eq!(plan.status(), Status::Extra);
    assert!(!plan.failed(), "an extra PBO is reported only");
    assert_eq!(plan.remote_only(), 1);
    assert_eq!(remote_targets(&plan), vec![format!("{DIR}/addons/Old.pbo")]);
}

#[test]
fn a_locked_extra_is_not_pruned() {
    let mut fixture = Fixture::new();
    fixture.remote.addons.push(remote_file("Old.pbo", 7));
    fixture.locked.insert("old.pbo".to_string());

    let plan = fixture.plan();
    assert!(matches!(plan.findings[0], Finding::Extra { .. }));
    assert!(plan.actions.is_empty(), "the rm action is omitted");
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.contains("listed in mods.lock; not pruned")),
        "{:?}",
        plan.warnings
    );
}

#[test]
fn force_prune_removes_a_locked_extra() {
    let mut fixture = Fixture::new();
    fixture.remote.addons.push(remote_file("Old.pbo", 7));
    fixture.locked.insert("old.pbo".to_string());
    fixture.force_prune = true;

    let plan = fixture.plan();
    assert_eq!(remote_targets(&plan), vec![format!("{DIR}/addons/Old.pbo")]);
}

#[test]
fn a_local_pbo_without_a_signature_warns() {
    let mut fixture = Fixture::new();
    fixture.local.signatures.clear();
    fixture.remote.addons.retain(|file| file.name != SIG);

    let plan = fixture.plan();
    assert!(plan.findings.is_empty(), "an unsigned PBO is not a finding");
    assert_eq!(plan.unsigned, 1);
    assert_eq!(plan.status(), Status::Unsigned);
    assert!(!plan.failed(), "a signature is optional by default");

    let strict = fixture.build(true);
    assert!(strict.failed(), "--require-signatures fails the pack");
}

#[test]
fn a_missing_remote_signature_is_uploaded() {
    let mut fixture = Fixture::new();
    fixture.remote.addons.retain(|file| file.name != SIG);

    let plan = fixture.plan();
    assert!(plan.findings.is_empty());
    assert_eq!(plan.status(), Status::Ok);
    assert_eq!(remote_targets(&plan), vec![format!("{DIR}/addons/{SIG}")]);
}

#[test]
fn keys_are_compared_and_uploaded() {
    let mut fixture = Fixture::new();
    fixture.remote.keys.clear();

    let plan = fixture.plan();
    assert_eq!(
        plan.findings[0],
        Finding::KeyMissing {
            name: KEY.to_string()
        }
    );
    assert_eq!(plan.status(), Status::Missing);
    assert_eq!(remote_targets(&plan), vec![format!("{DIR}/keys/{KEY}")]);

    fixture.remote.keys.push(remote_file(KEY, 40));
    let plan = fixture.plan();
    assert_eq!(
        plan.findings[0],
        Finding::KeyStale {
            name: KEY.to_string(),
            local: 50,
            remote: 40
        }
    );
    assert_eq!(remote_targets(&plan), vec![format!("{DIR}/keys/{KEY}")]);
}

#[test]
fn a_remote_only_key_is_reported_but_not_removed() {
    let mut fixture = Fixture::new();
    fixture.remote.keys.push(remote_file("Old.bikey", 3));

    let plan = fixture.plan();
    assert_eq!(
        plan.findings[0],
        Finding::KeyExtra {
            name: "Old.bikey".to_string()
        }
    );
    assert!(!plan.failed());
    assert!(plan.actions.is_empty(), "a key is never removed");
}

#[test]
fn a_meta_mismatch_is_reported_and_repaired() {
    let mut fixture = Fixture::new();
    fixture.remote.meta_id = Some("1".to_string());

    let plan = fixture.plan();
    assert_eq!(
        plan.findings[0],
        Finding::MetaMismatch {
            remote: Some("1".to_string())
        }
    );
    assert_eq!(plan.status(), Status::MetaMismatch);
    assert_eq!(remote_targets(&plan), vec![format!("{DIR}/meta.cpp")]);
}

#[test]
fn a_missing_mod_cpp_is_reported_and_uploaded() {
    let mut fixture = Fixture::new();
    fixture.remote.mod_size = None;

    let plan = fixture.plan();
    assert_eq!(plan.findings[0], Finding::ModCppMissing);
    assert_eq!(remote_targets(&plan), vec![format!("{DIR}/mod.cpp")]);
}

#[test]
fn a_missing_pack_folder_uploads_everything() {
    let mut fixture = Fixture::new();
    fixture.remote = RemotePack::default();

    let plan = fixture.plan();
    assert_eq!(plan.findings[0], Finding::NoPack);
    assert_eq!(plan.status(), Status::NoPack);
    assert!(plan.failed());
    let targets = remote_targets(&plan);
    assert!(targets.contains(&format!("{DIR}/addons/{PBO}")));
    assert!(targets.contains(&format!("{DIR}/meta.cpp")));
    assert!(targets.contains(&format!("{DIR}/mod.cpp")));
    assert!(targets.contains(&format!("{DIR}/keys/{KEY}")));
}

#[test]
fn a_pack_folder_is_never_removed_and_only_pbos_are() {
    let mut fixture = Fixture::new();
    fixture.remote.addons.push(remote_file("stray.bisign", 1));
    fixture.remote.addons.push(remote_file("old.PBO", 2));

    let plan = fixture.plan();
    assert_eq!(remote_targets(&plan), vec![format!("{DIR}/addons/old.PBO")]);
}

#[test]
fn a_symlink_on_the_server_is_not_written() {
    let mut fixture = Fixture::new();
    fixture.remote.addons.clear();
    fixture.remote.addons_symlinks.push(PBO.to_string());

    let plan = fixture.plan();
    assert!(!targets_contain(&plan, PBO));
    assert!(plan.warnings.iter().any(|w| w.contains("symlink")));
}

fn targets_contain(plan: &Plan, name: &str) -> bool {
    remote_targets(plan).iter().any(|t| t.ends_with(name))
}

#[test]
fn signature_owner_finds_the_pbo_name() {
    assert_eq!(signature_owner(SIG), Some(PBO));
    assert_eq!(signature_owner("uksfta_main.pbo"), None);
    assert_eq!(signature_owner("other.bisign"), None);
}

#[test]
fn signatures_pair_ignoring_case() {
    assert!(is_signature_of("UKSFTA_MAIN.PBO.KEY.BISIGN", PBO));
    assert!(is_signature_of(SIG, "UKSFTA_MAIN.PBO"));
    assert!(!is_signature_of(SIG, "other.pbo"));
}

#[test]
fn extension_helpers_ignore_case() {
    assert!(is_pbo("A.PBO"));
    assert!(is_bisign("A.pbo.KEY.BISIGN"));
    assert!(is_bikey("A.BIKEY"));
    assert!(!is_pbo("A.bisign"));
}

#[test]
fn upload_bytes_sums_every_upload_action() {
    let mut fixture = Fixture::new();
    fixture.remote.addons.clear();

    let plan = fixture.plan();
    assert_eq!(plan.upload_bytes(), 110, "the PBO and its signature");
}

#[test]
fn a_generated_meta_cpp_adds_no_upload_bytes() {
    let mut fixture = Fixture::new();
    fixture.remote.meta_id = Some("1".to_string());

    let plan = fixture.plan();
    assert!(plan.uploads().any(|action| matches!(
        action,
        Action::Upload { name, size, .. } if name == "meta.cpp" && *size == 0
    )));
    assert_eq!(plan.upload_bytes(), 0);
}
