//! The machine-readable report and the doctor report.

use serde::Serialize;

use super::config::Settings;
use super::plan::{Action, Finding, Plan};
use super::report::upload_units;

/// Writer for `--json` on `uksfta server`.
#[derive(Serialize)]
pub struct JsonReport {
    pub schema_version: u32,
    pub pack: Option<PackJson>,
    pub server: Option<ServerJson>,
    pub status: Option<String>,
    pub counts: CountsJson,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploads: Option<Vec<UploadJson>>,
    pub findings: Vec<FindingJson>,
    pub actions: Vec<ActionJson>,
    pub timings: TimingsJson,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_probe: Option<ProbeJson>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment: Option<SegmentJson>,
}

/// The effective segmented settings. Present only when the feature is on.
#[derive(Serialize)]
pub struct SegmentJson {
    pub enabled: bool,
    pub adaptive: bool,
    pub min_bytes: u64,
    pub connections: u8,
    pub chunk: Option<u32>,
    pub resume: String,
    pub reconnects: u8,
}

/// One planned upload and how it was sent.
#[derive(Serialize)]
pub struct UploadJson {
    pub name: String,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connections: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mb_per_second: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connections_max: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connections_start: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connections_peak: Option<u8>,
}

/// The link probe and the path it chose.
#[derive(Serialize)]
pub struct ProbeJson {
    pub system_mbps: f64,
    pub system_jobs: u8,
    pub native_one_mbps: f64,
    pub native_two_mbps: f64,
    pub read_mbps: f64,
    pub whole_seconds: f64,
    pub segmented_seconds: f64,
    pub decision: String,
}

impl From<&super::native::probe::Decision> for ProbeJson {
    fn from(decision: &super::native::probe::Decision) -> Self {
        Self {
            system_mbps: decision.probe.system_mbps,
            system_jobs: decision.probe.system_jobs,
            native_one_mbps: decision.probe.native_one_mbps,
            native_two_mbps: decision.probe.native_two_mbps,
            read_mbps: decision.probe.read_mbps,
            whole_seconds: decision.whole_seconds,
            segmented_seconds: decision.segmented_seconds,
            decision: if decision.segmented {
                "segmented".to_string()
            } else {
                "whole".to_string()
            },
        }
    }
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
            segment: settings.segment.enabled.then(|| SegmentJson {
                enabled: true,
                adaptive: settings.segment.adaptive,
                min_bytes: settings.segment.min_bytes,
                connections: settings.segment.connections,
                chunk: settings.segment.chunk,
                resume: settings.segment.resume.as_str().to_string(),
                reconnects: settings.segment.reconnects,
            }),
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        settings: Option<&Settings>,
        pack: Option<(String, String)>,
        plan: Option<&Plan>,
        timings: TimingsJson,
        segment_choice: Option<(&str, u8)>,
        segment_result: Option<(&str, &str)>,
        segment_counts: Option<(u8, u8, u8)>,
        segment_probe: Option<&super::native::probe::Decision>,
        sent: &[super::summary::SentFile],
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
        let uploads = settings
            .filter(|settings| settings.segment.enabled)
            .map(|_| {
                let measured = |remote: &str| sent.iter().find(|file| file.remote == remote);
                let mut entries = Vec::new();
                let mut planned: Vec<String> = Vec::new();
                if let Some(plan) = plan {
                    for unit in upload_units(plan) {
                        let selected =
                            segment_choice.filter(|(remote, _)| *remote == unit.remote.as_str());
                        let published =
                            segment_result.filter(|(remote, _)| *remote == unit.remote.as_str());
                        let file = measured(&unit.remote);
                        let counts = if selected.is_some() {
                            segment_counts
                        } else {
                            None
                        };
                        entries.push(UploadJson {
                            name: basename(&unit.remote),
                            mode: if selected.is_some() {
                                "segmented"
                            } else {
                                "whole"
                            }
                            .to_string(),
                            connections: selected.map(|(_, connections)| connections),
                            sha256: published
                                .map(|(_, hash)| (*hash).to_string())
                                .or_else(|| file.map(|file| file.sha256.clone())),
                            bytes: Some(unit.size),
                            seconds: file.map(|file| file.seconds),
                            mb_per_second: file.and_then(|file| file.rate_mbps),
                            estimated: file.map(|file| file.estimated),
                            remote: Some(unit.remote.clone()),
                            connections_max: counts.map(|c| c.0),
                            connections_start: counts.map(|c| c.1),
                            connections_peak: counts.map(|c| c.2),
                        });
                        planned.push(unit.remote.clone());
                    }
                }
                // A published segmented file leaves the plan once its hash
                // matches, so the result supplies the entry the plan no
                // longer holds.
                if let Some((remote, hash)) = segment_result {
                    if !planned.iter().any(|path| path == remote) {
                        let file = measured(remote);
                        entries.push(UploadJson {
                            name: basename(remote),
                            mode: "segmented".to_string(),
                            connections: segment_choice
                                .filter(|(chosen, _)| *chosen == remote)
                                .map(|(_, connections)| connections),
                            sha256: Some(hash.to_string()),
                            bytes: file.map(|file| file.bytes),
                            seconds: file.map(|file| file.seconds),
                            mb_per_second: file.and_then(|file| file.rate_mbps),
                            estimated: file.map(|file| file.estimated),
                            remote: Some(remote.to_string()),
                            connections_max: segment_counts.map(|c| c.0),
                            connections_start: segment_counts.map(|c| c.1),
                            connections_peak: segment_counts.map(|c| c.2),
                        });
                    }
                }
                // A sent file leaves the plan once it matches, so the
                // measured row supplies the entry the plan no longer holds.
                for file in sent {
                    if planned.iter().any(|path| path == &file.remote)
                        || entries
                            .iter()
                            .any(|entry| entry.remote.as_deref() == Some(file.remote.as_str()))
                    {
                        continue;
                    }
                    let segmented =
                        segment_result.is_some_and(|(remote, _)| remote == file.remote.as_str());
                    entries.push(UploadJson {
                        name: basename(&file.remote),
                        mode: if segmented { "segmented" } else { "whole" }.to_string(),
                        connections: segment_choice
                            .filter(|(remote, _)| *remote == file.remote)
                            .map(|(_, connections)| connections),
                        sha256: Some(file.sha256.clone()),
                        bytes: Some(file.bytes),
                        seconds: Some(file.seconds),
                        mb_per_second: file.rate_mbps,
                        estimated: Some(file.estimated),
                        remote: Some(file.remote.clone()),
                        connections_max: None,
                        connections_start: None,
                        connections_peak: None,
                    });
                }
                entries
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
            uploads,
            findings,
            actions,
            timings,
            segment_probe: segment_probe.map(ProbeJson::from),
            result: result_name.to_string(),
            error: result.as_ref().err().map(ToString::to_string),
        }
    }
}

fn basename(remote: &str) -> String {
    remote.rsplit('/').next().unwrap_or(remote).to_string()
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
