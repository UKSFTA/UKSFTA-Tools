//! The machine-readable report and the doctor report.

use serde::Serialize;

use super::config::Settings;
use super::plan::{Action, Finding, Plan};

/// Writer for `--json` on `uksfta server`.
#[derive(Serialize)]
pub struct JsonReport {
    pub schema_version: u32,
    pub pack: Option<PackJson>,
    pub server: Option<ServerJson>,
    pub status: Option<String>,
    pub counts: CountsJson,
    pub findings: Vec<FindingJson>,
    pub actions: Vec<ActionJson>,
    pub timings: TimingsJson,
    pub result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct PackJson {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct ServerJson {
    pub destination: String,
    pub port: u16,
    pub root: String,
    pub known_hosts: String,
    pub host_key_policy: String,
    pub buffer: u32,
    pub requests: u16,
}

impl From<&Settings> for ServerJson {
    fn from(settings: &Settings) -> Self {
        Self {
            destination: settings.destination.clone(),
            port: settings.port,
            root: settings.root.clone(),
            known_hosts: settings.known_hosts.display().to_string(),
            host_key_policy: settings.host_key_policy.as_str().to_string(),
            buffer: settings.transfer.buffer,
            requests: settings.transfer.requests,
        }
    }
}

#[derive(Serialize, Default)]
pub struct CountsJson {
    pub local_pbos: usize,
    pub local_signatures: usize,
    pub local_keys: usize,
    pub remote_pbos: usize,
    pub remote_keys: usize,
    pub unsigned: usize,
    pub upload_files: usize,
    pub upload_bytes: u64,
    pub problems: usize,
}

#[derive(Serialize, Default)]
pub struct FindingJson {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<String>,
}

#[derive(Serialize, Default)]
pub struct ActionJson {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Serialize, Default)]
pub struct TimingsJson {
    pub connect_ms: u64,
    pub deep_ms: Option<u64>,
    pub upload_ms: Option<u64>,
    pub total_ms: u64,
}

impl JsonReport {
    pub fn new(
        settings: Option<&Settings>,
        pack: Option<(String, String)>,
        plan: Option<&Plan>,
        timings: TimingsJson,
        result: &Result<(), crate::error::UksftaError>,
    ) -> Self {
        let counts = plan.map_or_else(CountsJson::default, |plan| CountsJson {
            local_pbos: plan.local_pbos,
            local_signatures: plan.local_signatures,
            local_keys: plan.local_keys,
            remote_pbos: plan.remote_pbos,
            remote_keys: plan.remote_keys,
            unsigned: plan.unsigned,
            upload_files: plan.uploads().count(),
            upload_bytes: plan.upload_bytes(),
            problems: plan.problems(),
        });
        let findings = plan.map_or_else(Vec::new, |plan| {
            plan.findings.iter().map(finding_json).collect()
        });
        let actions = plan.map_or_else(Vec::new, |plan| {
            plan.actions.iter().map(action_json).collect()
        });
        let status = plan.map(|plan| plan.status().as_str().to_string());
        let result_name = match result {
            Ok(()) => "ok",
            Err(crate::error::UksftaError::Check(_)) => "check-failed",
            Err(_) => "input-error",
        };
        Self {
            schema_version: 1,
            pack: pack.map(|(id, name)| PackJson { id, name }),
            server: settings.map(ServerJson::from),
            status,
            counts,
            findings,
            actions,
            timings,
            result: result_name.to_string(),
            error: result.as_ref().err().map(ToString::to_string),
        }
    }
}

fn finding_json(finding: &Finding) -> FindingJson {
    let mut out = FindingJson::default();
    match finding {
        Finding::NoPack => out.kind = "no-pack".to_string(),
        Finding::MetaMismatch { remote } => {
            out.kind = "meta-mismatch".to_string();
            out.remote_id = remote.clone();
        }
        Finding::Missing { name } => {
            out.kind = "missing".to_string();
            out.name = Some(name.clone());
        }
        Finding::StaleSize {
            name,
            local,
            remote,
        } => {
            out.kind = "stale-size".to_string();
            out.name = Some(name.clone());
            out.local = Some(*local);
            out.remote = Some(*remote);
        }
        Finding::StaleHash { name } => {
            out.kind = "stale-hash".to_string();
            out.name = Some(name.clone());
        }
        Finding::Extra { name } => {
            out.kind = "extra".to_string();
            out.name = Some(name.clone());
        }
        Finding::KeyMissing { name } => {
            out.kind = "key-missing".to_string();
            out.name = Some(name.clone());
        }
        Finding::KeyStale {
            name,
            local,
            remote,
        } => {
            out.kind = "key-stale".to_string();
            out.name = Some(name.clone());
            out.local = Some(*local);
            out.remote = Some(*remote);
        }
        Finding::KeyExtra { name } => {
            out.kind = "key-extra".to_string();
            out.name = Some(name.clone());
        }
        Finding::KeyAuthorityMissing {
            signature,
            authority,
        } => {
            out.kind = "key-authority-missing".to_string();
            out.signature = Some(signature.clone());
            out.authority = Some(authority.clone());
        }
        Finding::ModCppMissing => out.kind = "mod-cpp-missing".to_string(),
        Finding::ModCppStale { local, remote } => {
            out.kind = "mod-cpp-stale".to_string();
            out.local = Some(*local);
            out.remote = Some(*remote);
        }
    }
    out
}

fn action_json(action: &Action) -> ActionJson {
    match action {
        Action::Upload {
            name,
            local,
            remote,
            size,
        } => ActionJson {
            kind: "upload".to_string(),
            name: Some(name.clone()),
            local: Some(local.display().to_string()),
            remote: Some(remote.clone()),
            size: Some(*size),
        },
        Action::Remove { remote } => ActionJson {
            kind: "remove".to_string(),
            remote: Some(remote.clone()),
            ..ActionJson::default()
        },
    }
}

/// The doctor report.
#[derive(Serialize)]
pub struct DoctorReport {
    pub schema_version: u32,
    pub mode: String,
    pub pack: Option<PackJson>,
    pub server: Option<ServerJson>,
    pub checks: Vec<DoctorCheckJson>,
    pub free_space_bytes: Option<u64>,
    pub timings: DoctorTimingsJson,
    pub result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct DoctorCheckJson {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Serialize)]
pub struct DoctorTimingsJson {
    pub connect_ms: u64,
    pub total_ms: u64,
}

impl DoctorReport {
    pub fn new(
        settings: Option<&Settings>,
        pack: Option<(String, String)>,
        checks: Vec<DoctorCheckJson>,
        free_space: Option<u64>,
        connect_ms: u64,
        total_ms: u64,
        result: &Result<(), crate::error::UksftaError>,
    ) -> Self {
        Self {
            schema_version: 1,
            mode: "doctor".to_string(),
            pack: pack.map(|(id, name)| PackJson { id, name }),
            server: settings.map(ServerJson::from),
            checks,
            free_space_bytes: free_space,
            timings: DoctorTimingsJson {
                connect_ms,
                total_ms,
            },
            result: if result.is_ok() { "ok" } else { "input-error" }.to_string(),
            error: result.as_ref().err().map(ToString::to_string),
        }
    }
}
