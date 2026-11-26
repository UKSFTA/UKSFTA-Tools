//! The native SSH/SFTP transport and segmented upload.
//!
//! The segmented writer, atomic publish and progress live here. Resume and
//! checkpoints arrive in Wave 3.

pub mod connect;
pub mod known_hosts;
pub mod progress;
pub mod publish;
pub mod segment;
pub mod verify;
pub mod writer;

#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::Arc;

use openssh_sftp_client::Sftp;
use tokio::task::JoinSet;

use self::progress::SegmentedProgress;
use self::segment::SegmentPlan;
use crate::error::UksftaError;
use crate::server::config::Settings;
use crate::server::report::UploadUnit;

/// Upload one file over several connections, verify it and publish it.
/// Returns the verified SHA-256 of the file.
///
/// # Errors
/// Returns `Input` when the plan, a write, the length check, the read-back
/// hash or the publish fails. The temp file is removed on every failure.
pub fn run(
    settings: &Settings,
    state_root: &Path,
    pack_id: &str,
    unit: &UploadUnit,
    show_progress: bool,
) -> Result<String, UksftaError> {
    let connections = settings.segment.connections.max(1);
    let threads = usize::from(connections.min(4)).max(1);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(threads)
        .enable_all()
        .build()
        .map_err(UksftaError::Io)?;
    runtime.block_on(async { transfer(settings, state_root, pack_id, unit, show_progress).await })
}

async fn transfer(
    settings: &Settings,
    state_root: &Path,
    pack_id: &str,
    unit: &UploadUnit,
    show_progress: bool,
) -> Result<String, UksftaError> {
    let plan = SegmentPlan::new(
        unit.size,
        settings.segment.connections,
        settings.segment.chunk,
    )?;
    let run_id = writer::run_id();
    let temp = writer::plan_temp_name(&unit.remote);
    writer::assert_not_loadable(&temp)?;
    let aside_probe = writer::plan_aside_name(&unit.remote, &run_id);
    writer::assert_not_loadable(&aside_probe)?;

    let _lock = writer::take_run_lock(state_root, pack_id)?;
    let coordinator = connect::connect(settings).await?;
    let client = coordinator.sftp;

    let result = transfer_inner(
        settings,
        &client,
        unit,
        &plan,
        &temp,
        &run_id,
        show_progress,
    )
    .await;
    if result.is_err() {
        let mut fs = client.fs();
        let _ = fs.remove_file(&temp).await;
    }
    result
}

async fn transfer_inner(
    settings: &Settings,
    client: &Sftp,
    unit: &UploadUnit,
    plan: &SegmentPlan,
    temp: &str,
    run_id: &str,
    show_progress: bool,
) -> Result<String, UksftaError> {
    writer::create_temp(client, temp).await?;
    let queue = writer::queue(&plan.chunks);
    let progress = SegmentedProgress::new(
        unit.size,
        usize::from(settings.segment.connections),
        show_progress,
    );
    let buffer = settings.transfer.buffer;

    let mut tasks = JoinSet::new();
    for worker in 0..usize::from(settings.segment.connections.max(1)) {
        let settings = settings.clone();
        let queue = Arc::clone(&queue);
        let progress = Arc::clone(&progress);
        let temp = temp.to_string();
        let local = unit.local.clone();
        tasks.spawn(async move {
            let session = connect::connect(&settings).await?;
            let mut file = writer::open_range_writer(&session.sftp, &temp).await?;
            loop {
                let next = {
                    let mut guard = queue.lock().map_err(|_| {
                        UksftaError::Input("the range queue is poisoned".to_string())
                    })?;
                    guard.pop_front()
                };
                let Some(chunk) = next else {
                    break;
                };
                let mut advance = |bytes: u64| progress.advance(worker, bytes);
                writer::write_range(&mut file, &chunk, &local, buffer, &mut advance).await?;
            }
            file.close()
                .await
                .map_err(|error| UksftaError::Input(format!("sftp: {error}")))?;
            Ok::<(), UksftaError>(())
        });
    }

    let mut failure = None;
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                failure = Some(error);
                tasks.abort_all();
                break;
            }
            Err(error) => {
                failure = Some(UksftaError::Input(format!(
                    "a segmented worker stopped: {error}"
                )));
                tasks.abort_all();
                break;
            }
        }
    }
    if let Some(error) = failure {
        progress.finish();
        return Err(error);
    }
    progress.finish();

    writer::finish(client, temp, unit.size).await?;

    let hash = verify::hash_remote(client, temp, unit.size, buffer).await?;
    let local = crate::util::file_sha256(&unit.local).ok_or_else(|| {
        UksftaError::Input(format!(
            "cannot hash the local file {}",
            unit.local.display()
        ))
    })?;
    if hash != local {
        return Err(UksftaError::Input(
            "the server received different bytes; nothing was published".to_string(),
        ));
    }

    let mtime = std::fs::metadata(&unit.local)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs());
    publish::publish(client, temp, &unit.remote, run_id, mtime).await?;
    Ok(hash)
}
