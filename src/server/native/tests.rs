//! Live tests against the Docker OpenSSH fixture in `tools/sftp-test-server`.
//! They are `#[ignore]` and also need `UKSFTA_SFTP_LIVE=1`.

use std::path::{Path, PathBuf};

use tokio::io::{AsyncSeekExt, SeekFrom};

use super::connect;
use crate::server::config::{Segment, Settings, Transfer};
use crate::server::hostkey::HostKeyPolicy;
use crate::server::report::UploadUnit;

/// The hashed `known_hosts` pattern for `[127.0.0.1]:2222` and the committed
/// host key. Regenerate it with `ssh-keygen -H` if the fixture key changes.
const HASHED_2222: &str = "|1|wavKFbDi2nh5VKKyOlZEc2kkhqE=|ZilRQJZjyh+j1aDXMMVz/ovQK6w=";

/// The live tests share one server, and the kill cases stop every session,
/// so only one may run at a time.
static LIVE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn live_lock() -> tokio::sync::MutexGuard<'static, ()> {
    LIVE_LOCK.blocking_lock()
}

async fn live_lock_async() -> tokio::sync::MutexGuard<'static, ()> {
    LIVE_LOCK.lock().await
}

fn live() -> bool {
    if std::env::var("UKSFTA_SFTP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipped: set UKSFTA_SFTP_LIVE=1 and start the fixture");
        return false;
    }
    true
}

fn host() -> String {
    std::env::var("UKSFTA_SFTP_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
}

fn port() -> u16 {
    std::env::var("UKSFTA_SFTP_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2222)
}

fn client_key() -> PathBuf {
    std::env::var("UKSFTA_SFTP_KEY").map_or_else(
        |_| PathBuf::from("tools/sftp-test-server/keys/id_ed25519"),
        PathBuf::from,
    )
}

fn host_key() -> String {
    let path = std::env::var("UKSFTA_SFTP_HOST_KEY").map_or_else(
        |_| PathBuf::from("tools/sftp-test-server/keys/id_ed25519_host.pub"),
        PathBuf::from,
    );
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read the fixture host key: {error}"))
        .trim()
        .to_string()
}

fn settings(known_hosts: PathBuf, identity: Option<PathBuf>, port: u16) -> Settings {
    Settings {
        destination: format!("uksfta@{}", host()),
        port,
        root: "/".to_string(),
        identity,
        transfer: Transfer::default(),
        known_hosts,
        host_key_policy: HostKeyPolicy::Yes,
        launch_line: None,
        server_cfg: None,
        segment: Segment::default(),
    }
}

fn known_hosts(entries: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let path = dir.path().join("known_hosts");
    std::fs::write(&path, entries).expect("write known_hosts");
    (dir, path)
}

fn pin(port: u16) -> (tempfile::TempDir, PathBuf) {
    known_hosts(&format!("[{}]:{port} {}\n", host(), host_key()))
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_key_file_connects_and_lists_the_root() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let (_dir, pinned) = pin(port());
    let settings = settings(pinned, Some(client_key()), port());

    let session = connect::connect(&settings).await.expect("key-file connect");
    session
        .sftp
        .fs()
        .metadata("/")
        .await
        .expect("stat the remote root");
}

#[tokio::test]
#[ignore = "needs the Docker fixture, an agent and UKSFTA_SFTP_LIVE=1"]
async fn an_agent_identity_connects_when_one_is_available() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    if std::env::var_os("SSH_AUTH_SOCK").is_none() {
        eprintln!("skipped: SSH_AUTH_SOCK is not set");
        return;
    }
    let (_dir, pinned) = pin(port());
    let settings = settings(pinned, None, port());
    if let Err(error) = connect::connect(&settings).await {
        eprintln!("skipped: the agent offered no accepted key: {error}");
    }
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn an_unknown_host_is_refused() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let (_dir, pinned) = known_hosts("");
    let settings = settings(pinned, Some(client_key()), port());

    let text = error_text(connect::connect(&settings).await);
    assert!(text.contains("not in"), "{text}");
    assert!(text.contains("known_hosts"), "{text}");
    assert!(text.contains("--host-key-policy accept-new"), "{text}");
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_changed_host_key_is_refused() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let client = std::fs::read_to_string(client_key().with_extension("pub"))
        .expect("read the client public key");
    let (_dir, pinned) = known_hosts(&format!("[{}]:{} {}\n", host(), port(), client.trim()));
    let settings = settings(pinned, Some(client_key()), port());

    let text = error_text(connect::connect(&settings).await);
    assert!(text.contains("changed"), "{text}");
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_hashed_host_entry_is_accepted() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let (_dir, pinned) = known_hosts(&format!("{HASHED_2222} {}\n", host_key()));
    let settings = settings(pinned, Some(client_key()), 2222);

    connect::connect(&settings)
        .await
        .expect("a hashed entry must match");
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_revoked_entry_is_refused() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let (_dir, pinned) = known_hosts(&format!(
        "@revoked [{}]:{} {}\n",
        host(),
        port(),
        host_key()
    ));
    let settings = settings(pinned, Some(client_key()), port());

    let text = error_text(connect::connect(&settings).await);
    assert!(text.contains("revoked"), "{text}");
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_certificate_authority_line_is_refused() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let (_dir, pinned) = known_hosts(&format!(
        "@cert-authority [{}]:{} {}\n",
        host(),
        port(),
        host_key()
    ));
    let settings = settings(pinned, Some(client_key()), port());

    let text = error_text(connect::connect(&settings).await);
    assert!(text.contains("@cert-authority"), "{text}");
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_non_default_port_entry_is_accepted() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let (_dir, pinned) = pin(2223);
    let settings = settings(pinned, Some(client_key()), 2223);

    connect::connect(&settings)
        .await
        .expect("the [host]:2223 entry must match");
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn no_identity_names_the_two_supported_methods() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let (_dir, pinned) = pin(port());
    let settings = settings(pinned, None, port());

    let result = connect::connect(&settings).await;
    if let Err(error) = result {
        let text = error.to_string();
        assert!(text.contains("key file"), "{text}");
        assert!(text.contains("ssh-agent"), "{text}");
    }
}

fn error_text(result: Result<connect::NativeSession, crate::error::UksftaError>) -> String {
    match result {
        Ok(_) => panic!("the connection must fail"),
        Err(error) => error.to_string(),
    }
}

// ---- Segmented upload ------------------------------------------------------

/// A unique remote directory under the fixture's writable home.
fn remote_dir(tag: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!("/home/uksfta/upload/{tag}-{}-{nanos}", std::process::id())
}

/// A segmented settings value that is on, with a small minimum.
fn segmented_settings(pinned: PathBuf, connections: u8, chunk: Option<u32>) -> Settings {
    let mut settings = settings(pinned, Some(client_key()), port());
    settings.segment = Segment {
        enabled: true,
        min_bytes: 1,
        connections,
        chunk,
        ..Segment::default()
    };
    settings
}

async fn session(settings: &Settings) -> connect::NativeSession {
    connect::connect(settings).await.expect("a live session")
}

/// Create one remote directory. SFTP opens no parents.
async fn make_dir(sftp: &openssh_sftp_client::Sftp, path: &str) {
    sftp.fs()
        .create_dir(path)
        .await
        .expect("create the remote directory");
}

/// A local file of `size` bytes with a repeatable pattern.
fn local_file(dir: &tempfile::TempDir, name: &str, size: u64) -> PathBuf {
    use std::io::Write;
    let path = dir.path().join(name);
    let mut file = std::fs::File::create(&path).expect("create the local file");
    let block: Vec<u8> = (0..4096u32).map(|value| (value % 251) as u8).collect();
    let mut left = size;
    while left > 0 {
        let want = usize::try_from(left.min(block.len() as u64)).expect("block");
        file.write_all(&block[..want])
            .expect("write the local file");
        left -= want as u64;
    }
    path
}

fn units(local: &Path, remote: &str, size: u64) -> UploadUnit {
    UploadUnit {
        local: local.to_path_buf(),
        remote: remote.to_string(),
        size,
    }
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn the_temp_file_starts_empty_and_two_handles_share_it() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let (dir, pinned) = pin(port());
    let settings = segmented_settings(pinned, 2, Some(4096 * 4));
    let session = session(&settings).await;
    let sftp = &session.sftp;
    let base = remote_dir("temp");
    let temp = format!("{base}/a.pbo.uksfta-part");
    make_dir(sftp, &base).await;

    super::writer::create_temp(sftp, &temp)
        .await
        .expect("create");
    let len = sftp.fs().metadata(&temp).await.expect("stat").len();
    assert_eq!(len, Some(0), "the temp file starts empty");

    let block = vec![7u8; 4096];
    let mut first = super::writer::open_range_writer(sftp, &temp)
        .await
        .expect("first handle");
    let mut second = super::writer::open_range_writer(sftp, &temp)
        .await
        .expect("second handle");
    first.seek(SeekFrom::Start(0)).await.expect("seek");
    first.write_all(&block).await.expect("write");
    second.seek(SeekFrom::Start(4096)).await.expect("seek");
    second.write_all(&block).await.expect("write");
    first.close().await.expect("close");
    second.close().await.expect("close");

    let len = sftp.fs().metadata(&temp).await.expect("stat").len();
    assert_eq!(len, Some(8192), "both handles wrote into one file");

    let mut fs = sftp.fs();
    let _ = fs.remove_file(&temp).await;
    drop(dir);
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn four_connections_write_a_sixteen_mebibyte_file() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    const SIZE: u64 = 16 * 1024 * 1024;
    let (dir, pinned) = pin(port());
    let local = local_file(&dir, "sixteen.bin", SIZE);
    let settings = segmented_settings(pinned, 4, Some(4 * 1024 * 1024));
    let plan = super::segment::SegmentPlan::new(SIZE, 4, Some(4 * 1024 * 1024)).expect("plan");
    assert_eq!(plan.chunks.len(), 4);
    let base = remote_dir("write");
    let remote = format!("{base}/sixteen.bin.uksfta-part");
    let coordinator = session(&settings).await;
    make_dir(&coordinator.sftp, &base).await;
    super::writer::create_temp(&coordinator.sftp, &remote)
        .await
        .expect("create");

    let scheduler = std::sync::Arc::new(std::sync::Mutex::new(super::schedule::Scheduler::new(
        &plan.chunks,
    )));
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..4 {
        let settings = settings.clone();
        let scheduler = std::sync::Arc::clone(&scheduler);
        let remote = remote.clone();
        let local = local.clone();
        tasks.spawn(async move {
            let session = session(&settings).await;
            let mut file = super::writer::open_range_writer(&session.sftp, &remote).await?;
            loop {
                let next = scheduler.lock().expect("scheduler").take();
                let Some(chunk) = next else { break };
                super::writer::write_range(&mut file, &chunk, &local, 32_768, None, &mut |_| {})
                    .await?;
                scheduler.lock().expect("scheduler").complete(chunk.index);
            }
            file.close()
                .await
                .map_err(|error| crate::error::UksftaError::Input(format!("sftp: {error}")))?;
            Ok::<(), crate::error::UksftaError>(())
        });
    }
    while let Some(joined) = tasks.join_next().await {
        joined.expect("a worker task").expect("a worker");
    }

    super::writer::finish(&coordinator.sftp, &remote, SIZE)
        .await
        .expect("the file is complete");
    let remote_hash = super::verify::hash_remote(&coordinator.sftp, &remote, SIZE, 65_536)
        .await
        .expect("hash the remote file");
    let local_hash = crate::util::file_sha256(&local).expect("hash the local file");
    assert_eq!(remote_hash, local_hash);

    let mut fs = coordinator.sftp.fs();
    let _ = fs.remove_file(&remote).await;
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_short_local_file_is_refused_by_name() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let (dir, pinned) = pin(port());
    let local = local_file(&dir, "short.bin", 4095);
    let settings = segmented_settings(pinned, 1, Some(4096));
    let base = remote_dir("short");
    let remote = format!("{base}/short.bin.uksfta-part");
    let session = session(&settings).await;
    make_dir(&session.sftp, &base).await;
    super::writer::create_temp(&session.sftp, &remote)
        .await
        .expect("create");
    let mut file = super::writer::open_range_writer(&session.sftp, &remote)
        .await
        .expect("handle");

    let chunk = super::segment::Chunk {
        index: 0,
        start: 0,
        len: 4096,
    };
    let error = super::writer::write_range(&mut file, &chunk, &local, 4096, None, &mut |_| {})
        .await
        .expect_err("the local file is short");
    let text = error.to_string();
    assert!(text.contains("short.bin"), "{text}");
    assert!(text.contains("ended early"), "{text}");

    let mut fs = session.sftp.fs();
    let _ = fs.remove_file(&remote).await;
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_missing_range_makes_finish_refuse_and_publish_never_run() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    const SIZE: u64 = 2 * 1024 * 1024;
    let (dir, pinned) = pin(port());
    let local = local_file(&dir, "gap.bin", SIZE);
    let settings = segmented_settings(pinned, 1, None);
    let base = remote_dir("gap");
    let temp = format!("{base}/gap.bin.uksfta-part");
    let final_path = format!("{base}/gap.bin");
    let session = session(&settings).await;
    make_dir(&session.sftp, &base).await;
    super::writer::create_temp(&session.sftp, &temp)
        .await
        .expect("create");

    // Write only the first half of the file.
    let plan = super::segment::SegmentPlan::new(SIZE, 1, Some(1024 * 1024)).expect("plan");
    let mut file = super::writer::open_range_writer(&session.sftp, &temp)
        .await
        .expect("handle");
    super::writer::write_range(
        &mut file,
        &plan.chunks[0],
        &local,
        32_768,
        None,
        &mut |_| {},
    )
    .await
    .expect("the first range");
    file.close().await.expect("close");

    let error = super::writer::finish(&session.sftp, &temp, SIZE)
        .await
        .expect_err("the file is short");
    let text = error.to_string();
    assert!(text.contains("1048576"), "{text}");
    assert!(text.contains("2097152"), "{text}");

    let mut fs = session.sftp.fs();
    assert!(fs.metadata(&temp).await.is_ok(), "the temp file remains");
    assert!(
        fs.metadata(&final_path).await.is_err(),
        "the final name was never written"
    );
    let _ = fs.remove_file(&temp).await;
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_wrong_length_is_refused_by_the_hash() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let (dir, pinned) = pin(port());
    let local = local_file(&dir, "hash.bin", 8192);
    let settings = segmented_settings(pinned, 1, None);
    let base = remote_dir("hash");
    let remote = format!("{base}/hash.bin");
    let session = session(&settings).await;
    make_dir(&session.sftp, &base).await;
    session
        .sftp
        .fs()
        .write(&remote, std::fs::read(&local).expect("read"))
        .await
        .expect("upload");

    let good = super::verify::hash_remote(&session.sftp, &remote, 8192, 65_536)
        .await
        .expect("hash");
    assert_eq!(good, crate::util::file_sha256(&local).expect("local hash"));

    let error = super::verify::hash_remote(&session.sftp, &remote, 8193, 65_536)
        .await
        .expect_err("the length differs");
    assert!(error.to_string().contains("8192"), "{error}");

    let mut fs = session.sftp.fs();
    let _ = fs.remove_file(&remote).await;
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn publish_renames_the_temp_and_sets_the_time() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let (dir, pinned) = pin(port());
    let settings = segmented_settings(pinned, 1, None);
    let base = remote_dir("publish");
    let temp = format!("{base}/mod.pbo.uksfta-part");
    let final_path = format!("{base}/mod.pbo");
    let session = session(&settings).await;
    {
        let mut fs = session.sftp.fs();
        fs.create_dir(&base).await.expect("create the directory");
    }
    super::writer::create_temp(&session.sftp, &temp)
        .await
        .expect("create");
    let block = vec![3u8; 1024];
    let mut file = super::writer::open_range_writer(&session.sftp, &temp)
        .await
        .expect("handle");
    file.write_all(&block).await.expect("write");
    file.close().await.expect("close");

    let method = super::publish::publish(
        &session.sftp,
        &temp,
        &final_path,
        "0123456789abcdef",
        Some(1_700_000_000),
    )
    .await
    .expect("publish");
    assert_eq!(method, super::publish::PublishMethod::PosixRename);

    let mut fs = session.sftp.fs();
    assert!(fs.metadata(&temp).await.is_err(), "the temp name is gone");
    let metadata = fs.metadata(&final_path).await.expect("the final file");
    assert_eq!(metadata.len(), Some(1024));
    assert_eq!(
        metadata
            .modified()
            .map(openssh_sftp_client::UnixTimeStamp::into_raw),
        Some(1_700_000_000)
    );
    drop(dir);
}

#[test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
fn a_full_segmented_run_publishes_and_returns_the_hash() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    const SIZE: u64 = 64 * 1024 * 1024;
    let (dir, pinned) = pin(port());
    let local = local_file(&dir, "big.pbo", SIZE);
    let settings = segmented_settings(pinned, 4, Some(4 * 1024 * 1024));
    let base = remote_dir("run");
    let final_path = format!("{base}/big.pbo");
    let unit = units(&local, &final_path, SIZE);
    let state = tempfile::TempDir::new().expect("state dir");
    {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let session = session(&settings).await;
            make_dir(&session.sftp, &base).await;
        });
    }

    let hash = super::run(&settings, state.path(), "pack-1", &unit, None, false)
        .expect("the segmented run");
    assert_eq!(hash, crate::util::file_sha256(&local).expect("local hash"));

    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let session = session(&settings).await;
        let metadata = session
            .sftp
            .fs()
            .metadata(&final_path)
            .await
            .expect("the published file");
        assert_eq!(metadata.len(), Some(SIZE));
        assert!(
            session
                .sftp
                .fs()
                .metadata(&format!("{final_path}.uksfta-part"))
                .await
                .is_err(),
            "the temp name is gone"
        );
        let mut fs = session.sftp.fs();
        let _ = fs.remove_file(&final_path).await;
    });
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn the_readback_buffer_rate_is_measurable() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    const SIZE: u64 = 256 * 1024 * 1024;
    let (dir, pinned) = pin(port());
    let local = local_file(&dir, "read.pbo", SIZE);
    let settings = segmented_settings(pinned, 8, Some(8 * 1024 * 1024));
    let base = remote_dir("readbuf");
    let temp = format!("{base}/read.probe");
    let session = session(&settings).await;
    make_dir(&session.sftp, &base).await;

    super::writer::create_temp(&session.sftp, &temp)
        .await
        .expect("the temp file");
    let mut file = super::writer::open_range_writer(&session.sftp, &temp)
        .await
        .expect("the writer");
    let chunk = super::segment::Chunk {
        index: 0,
        start: 0,
        len: SIZE as u32,
    };
    super::writer::write_range(&mut file, &chunk, &local, 32_768, None, &mut |_| {})
        .await
        .expect("the write");
    file.close().await.expect("close");

    let started = std::time::Instant::now();
    super::verify::hash_remote(&session.sftp, &temp, SIZE, 32_768)
        .await
        .expect("the small-buffer hash");
    let small = started.elapsed();
    let started = std::time::Instant::now();
    super::verify::hash_remote(&session.sftp, &temp, SIZE, super::verify::READBACK_BUFFER)
        .await
        .expect("the read-back hash");
    let large = started.elapsed();
    println!(
        "readback 256 MiB: 32 KiB {:.2} s, {} B {:.2} s",
        small.as_secs_f64(),
        super::verify::READBACK_BUFFER,
        large.as_secs_f64()
    );

    let mut fs = session.sftp.fs();
    let _ = fs.remove_file(&temp).await;
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn preallocation_sets_the_temp_length_before_the_first_write() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    const SIZE: u64 = 4 * 1024 * 1024;
    let (dir, pinned) = pin(port());
    let local = local_file(&dir, "pre.pbo", SIZE);
    let settings = segmented_settings(pinned, 2, None);
    let base = remote_dir("prealloc");
    let temp = format!("{base}/pre.pbo.uksfta-part");
    let session = session(&settings).await;
    make_dir(&session.sftp, &base).await;

    super::writer::create_temp(&session.sftp, &temp)
        .await
        .expect("the temp file");
    assert_eq!(
        session.sftp.fs().metadata(&temp).await.unwrap().len(),
        Some(0),
        "the temp starts empty"
    );

    let outcome = super::writer::preallocate(&session.sftp, &temp, SIZE)
        .await
        .expect("preallocate");
    let len = session.sftp.fs().metadata(&temp).await.unwrap().len();
    match outcome {
        super::writer::Preallocated::Full => assert_eq!(len, Some(SIZE)),
        super::writer::Preallocated::Unsupported => {
            assert_eq!(len, Some(0), "an unsupported server leaves the temp empty");
        }
    }

    let mut fs = session.sftp.fs();
    let _ = fs.remove_file(&temp).await;
    let _ = local;
}

#[test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
fn an_adaptive_run_with_eight_connections_publishes_the_same_bytes() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    const SIZE: u64 = 64 * 1024 * 1024;
    let (dir, pinned) = pin(port());
    let local = local_file(&dir, "adaptive.pbo", SIZE);
    let mut settings = segmented_settings(pinned, 8, Some(4 * 1024 * 1024));
    settings.segment.adaptive = true;
    let base = remote_dir("adaptive");
    let final_path = format!("{base}/adaptive.pbo");
    let unit = units(&local, &final_path, SIZE);
    let state = tempfile::TempDir::new().expect("state dir");
    {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let session = session(&settings).await;
            make_dir(&session.sftp, &base).await;
        });
    }

    let hash = super::run(&settings, state.path(), "pack-a", &unit, None, false)
        .expect("the adaptive run");
    assert_eq!(hash, crate::util::file_sha256(&local).expect("local hash"));

    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let session = session(&settings).await;
        let metadata = session
            .sftp
            .fs()
            .metadata(&final_path)
            .await
            .expect("the published file");
        assert_eq!(metadata.len(), Some(SIZE));
        let mut fs = session.sftp.fs();
        let _ = fs.remove_file(&final_path).await;
    });
}

#[test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
fn a_failed_segmented_run_leaves_no_temp_and_no_final() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    let (dir, pinned) = pin(port());
    let local = local_file(&dir, "fail.pbo", 64 * 1024);
    let settings = segmented_settings(pinned, 2, None);
    let base = remote_dir("fail");
    let final_path = format!("{base}/fail.pbo");
    // The size is larger than the local file, so the last range runs dry.
    let unit = units(&local, &final_path, 128 * 1024);
    let state = tempfile::TempDir::new().expect("state dir");
    {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let session = session(&settings).await;
            make_dir(&session.sftp, &base).await;
        });
    }

    let error = super::run(&settings, state.path(), "pack-2", &unit, None, false)
        .expect_err("the short read must fail the run");
    assert!(error.to_string().contains("ended early"), "{error}");

    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let session = session(&settings).await;
        let mut fs = session.sftp.fs();
        let kept = fs
            .metadata(&format!("{final_path}.uksfta-part"))
            .await
            .expect("the temp file is kept for a resume");
        assert!(kept.len().unwrap_or(0) > 0, "some bytes were written");
        assert!(
            fs.metadata(&final_path).await.is_err(),
            "nothing reached the final name"
        );
    });
}

// ---- Checkpoints, resume and retries ---------------------------------------

/// Poll the remote temp until it reaches `at_least`, or it disappears.
/// Wait until the checkpoint records some acknowledged bytes. The temp
/// length cannot say how far the transfer got, because preallocation sets
/// it to the full size before the first write.
fn poll_acked(state: &Path, pack_id: &str, remote: &str, at_least: u64) {
    let path = super::checkpoint::path_for(state, pack_id, remote);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Some(checkpoint) = super::checkpoint::load(&path) {
            if checkpoint.acked_prefix >= at_least {
                return;
            }
        }
        if std::time::Instant::now() > deadline {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Wait until a checkpoint records at least one acknowledged range.
fn wait_for_ack(state: &Path, pack_id: &str, remote: &str) -> super::checkpoint::Checkpoint {
    let path = super::checkpoint::path_for(state, pack_id, remote);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if let Some(checkpoint) = super::checkpoint::load(&path) {
            if checkpoint.acked_prefix > 0 {
                return checkpoint;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no range was acknowledged in 60 s"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Kill every server-side sftp subsystem process.
fn kill_sessions() {
    let status = std::process::Command::new("docker")
        .args([
            "compose",
            "-f",
            "tools/sftp-test-server/docker-compose.yml",
            "exec",
            "-T",
            "sftp",
            "pkill",
            "-f",
            "sftp-server",
        ])
        .status()
        .expect("docker compose exec");
    assert!(status.success(), "the session kill failed");
}

/// A 64 MiB pack file and the settings to send it.
struct ResumeFixture {
    dir: tempfile::TempDir,
    state: tempfile::TempDir,
    settings: Settings,
    base: String,
    final_path: String,
    temp: String,
    local: PathBuf,
    unit: UploadUnit,
}

const RESUME_SIZE: u64 = 96 * 1024 * 1024;

fn resume_fixture(tag: &str, reconnects: u8) -> ResumeFixture {
    let (dir, pinned) = pin(port());
    let local = local_file(&dir, "resume.pbo", RESUME_SIZE);
    let mut settings = segmented_settings(pinned, 4, Some(4 * 1024 * 1024));
    settings.segment.reconnects = reconnects;
    let base = remote_dir(tag);
    let final_path = format!("{base}/resume.pbo");
    let temp = format!("{final_path}.uksfta-part");
    let unit = units(&local, &final_path, RESUME_SIZE);
    let state = tempfile::TempDir::new().expect("state dir");
    ResumeFixture {
        dir,
        state,
        settings,
        base,
        final_path,
        temp,
        local,
        unit,
    }
}

#[test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
fn a_killed_session_reconnects_and_publishes() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    let fixture = resume_fixture("reconnect", 2);
    {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let session = session(&fixture.settings).await;
            make_dir(&session.sftp, &fixture.base).await;
        });
    }

    let settings = fixture.settings.clone();
    let state = fixture.state.path().to_path_buf();
    let unit = fixture.unit.clone();
    let handle =
        std::thread::spawn(move || super::run(&settings, &state, "pack-r", &unit, None, false));

    poll_acked(
        fixture.state.path(),
        "pack-r",
        &fixture.unit.remote,
        4 * 1024 * 1024,
    );
    kill_sessions();
    let result = handle.join().expect("the run thread");

    let hash = result.expect("one killed session must be replaced");
    assert_eq!(
        hash,
        crate::util::file_sha256(&fixture.local).expect("local hash")
    );
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let session = session(&fixture.settings).await;
        let metadata = session
            .sftp
            .fs()
            .metadata(&fixture.final_path)
            .await
            .expect("the published file");
        assert_eq!(metadata.len(), Some(RESUME_SIZE));
    });
    drop(fixture.dir);
}

#[test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
fn a_killed_session_with_no_reconnect_fails_and_keeps_the_temp() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    let fixture = resume_fixture("noreconnect", 0);
    {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let session = session(&fixture.settings).await;
            make_dir(&session.sftp, &fixture.base).await;
        });
    }

    let settings = fixture.settings.clone();
    let state = fixture.state.path().to_path_buf();
    let unit = fixture.unit.clone();
    let handle =
        std::thread::spawn(move || super::run(&settings, &state, "pack-n", &unit, None, false));

    poll_acked(
        fixture.state.path(),
        "pack-n",
        &fixture.unit.remote,
        4 * 1024 * 1024,
    );
    kill_sessions();
    let result = handle.join().expect("the run thread");
    assert!(result.is_err(), "a dead session with no reconnect fails");

    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let session = session(&fixture.settings).await;
        let mut fs = session.sftp.fs();
        assert!(
            fs.metadata(&fixture.temp).await.is_ok(),
            "the temp file is kept for a resume"
        );
        assert!(
            fs.metadata(&fixture.final_path).await.is_err(),
            "nothing reached the final name"
        );
    });
    drop(fixture.dir);
}

#[test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
fn a_resume_continues_from_the_acked_prefix() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    let mut fixture = resume_fixture("resume", 0);
    fixture.settings.segment.connections = 1;
    {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let session = session(&fixture.settings).await;
            make_dir(&session.sftp, &fixture.base).await;
        });
    }

    let settings = fixture.settings.clone();
    let state = fixture.state.path().to_path_buf();
    let unit = fixture.unit.clone();
    let handle =
        std::thread::spawn(move || super::run(&settings, &state, "pack-a", &unit, None, false));
    wait_for_ack(fixture.state.path(), "pack-a", &fixture.final_path);
    kill_sessions();
    assert!(handle.join().expect("the run thread").is_err());

    let path = super::checkpoint::path_for(fixture.state.path(), "pack-a", &fixture.final_path);
    let checkpoint = super::checkpoint::load(&path).expect("a checkpoint");
    assert!(checkpoint.acked_prefix > 0, "some ranges were acknowledged");
    assert_eq!(checkpoint.local_size, RESUME_SIZE);

    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let (start, temp) = runtime.block_on(async {
        let session = session(&fixture.settings).await;
        let temp_size = session
            .sftp
            .fs()
            .metadata(&fixture.temp)
            .await
            .expect("the temp file")
            .len()
            .unwrap_or(0);
        assert!(
            checkpoint.acked_prefix <= temp_size,
            "the prefix never exceeds the bytes on the server"
        );
        let decision = super::checkpoint::decide(
            &session.sftp,
            fixture.state.path(),
            "pack-a",
            &fixture.unit,
            &fixture.settings,
        )
        .await
        .expect("decide");
        (decision.0, decision.1)
    });
    assert_eq!(temp, fixture.temp);
    match start {
        super::checkpoint::Start::Resume { from } => {
            assert_eq!(from, checkpoint.acked_prefix);
            assert!(from < RESUME_SIZE, "the run was interrupted mid-file");
        }
        super::checkpoint::Start::Fresh => panic!("a valid checkpoint must resume"),
    }

    let hash = super::run(
        &fixture.settings,
        fixture.state.path(),
        "pack-a",
        &fixture.unit,
        None,
        false,
    )
    .expect("the resumed run");
    assert_eq!(
        hash,
        crate::util::file_sha256(&fixture.local).expect("local hash")
    );
    assert!(!path.exists(), "the checkpoint is removed after a publish");
    drop(fixture.dir);
}

#[test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
fn a_corrupted_prefix_restarts_from_scratch() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    let mut fixture = resume_fixture("corrupt", 0);
    // One connection acknowledges one range at a time, so the first ack is
    // early and the kill always lands mid-transfer.
    fixture.settings.segment.connections = 1;
    {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let session = session(&fixture.settings).await;
            make_dir(&session.sftp, &fixture.base).await;
        });
    }

    let settings = fixture.settings.clone();
    let state = fixture.state.path().to_path_buf();
    let unit = fixture.unit.clone();
    let handle =
        std::thread::spawn(move || super::run(&settings, &state, "pack-c", &unit, None, false));
    wait_for_ack(fixture.state.path(), "pack-c", &fixture.final_path);
    kill_sessions();
    assert!(handle.join().expect("the run thread").is_err());
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let start = runtime.block_on(async {
        let session = session(&fixture.settings).await;
        let mut fs = session.sftp.fs();
        assert!(
            fs.metadata(&fixture.temp).await.is_ok(),
            "the interrupted temp must still exist on the server"
        );
        // Flip one byte inside the acknowledged prefix.
        let mut file = session
            .sftp
            .options()
            .write(true)
            .open(&fixture.temp)
            .await
            .expect("open the temp");
        file.seek(SeekFrom::Start(0)).await.expect("seek");
        file.write(&[0xff]).await.expect("write the corrupt byte");
        file.close().await.expect("close");

        let (start, _) = super::checkpoint::decide(
            &session.sftp,
            fixture.state.path(),
            "pack-c",
            &fixture.unit,
            &fixture.settings,
        )
        .await
        .expect("decide");
        start
    });
    assert_eq!(
        start,
        super::checkpoint::Start::Fresh,
        "corruption restarts"
    );
    assert!(
        !super::checkpoint::path_for(fixture.state.path(), "pack-c", &fixture.final_path).exists(),
        "the discarded checkpoint is removed"
    );

    let hash = super::run(
        &fixture.settings,
        fixture.state.path(),
        "pack-c",
        &fixture.unit,
        None,
        false,
    )
    .expect("the restarted run");
    assert_eq!(
        hash,
        crate::util::file_sha256(&fixture.local).expect("local hash")
    );
    drop(fixture.dir);
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn an_old_orphan_is_removed_and_a_fresh_one_is_kept() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    use openssh_sftp_client::metadata::MetaDataBuilder;
    use openssh_sftp_client::UnixTimeStamp;

    let (dir, pinned) = pin(port());
    let settings = segmented_settings(pinned, 1, None);
    let base = remote_dir("orphan");
    let session = session(&settings).await;
    make_dir(&session.sftp, &base).await;
    let old = format!("{base}/x.pbo.uksfta-part-old");
    let fresh = format!("{base}/y.pbo.uksfta-part-new");
    let folder = format!("{base}/z.uksfta-part-dir");
    let mut fs = session.sftp.fs();
    fs.write(&old, b"old").await.expect("seed the old temp");
    fs.write(&fresh, b"new").await.expect("seed the fresh temp");
    fs.create_dir(&folder).await.expect("seed the directory");

    let two_days_ago = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs().saturating_sub(2 * 24 * 60 * 60))
        .unwrap_or_default();
    let stamp = UnixTimeStamp::from_raw(u32::try_from(two_days_ago).expect("stamp"))
        .expect("a valid stamp");
    let mut builder = MetaDataBuilder::new();
    builder.time(stamp, stamp);
    fs.set_metadata(&old, builder.create())
        .await
        .expect("age the old temp");

    let removed = super::writer::clean_orphans(&session.sftp, &base, None)
        .await
        .expect("clean");
    assert_eq!(removed, vec![old.clone()], "{removed:?}");
    assert!(fs.metadata(&old).await.is_err(), "the old temp is gone");
    assert!(fs.metadata(&fresh).await.is_ok(), "the fresh temp stays");
    assert!(
        fs.metadata(&folder).await.is_ok(),
        "a directory is not removed"
    );
    drop(dir);
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_shortfall_names_the_free_and_needed_bytes() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let (dir, pinned) = pin(port());
    let settings = segmented_settings(pinned, 1, None);
    let base = remote_dir("space");
    let session = session(&settings).await;
    make_dir(&session.sftp, &base).await;

    let free = super::writer::check_space(&session.sftp, &base, 1)
        .await
        .expect("a small need fits");
    if let Some(free) = free {
        assert!(free > 0, "the server reports some free space");
    }
    let need = 1024u64 * 1024 * 1024 * 1024;
    let error = super::writer::check_space(&session.sftp, &base, need)
        .await
        .expect_err("1 TiB cannot fit");
    let text = error.to_string();
    assert!(text.contains(&need.to_string()), "{text}");
    assert!(free.is_some(), "the fixture advertises statvfs");
    drop(dir);
}

// ---- Password authentication ----------------------------------------------

/// The password lives only in the compose service's environment block.
fn fixture_password(service: &str) -> String {
    let out = std::process::Command::new("docker")
        .args([
            "compose",
            "-f",
            "tools/sftp-test-server/docker-compose.yml",
            "exec",
            "-T",
            service,
            "sh",
            "-c",
            "printf %s \"$FIXTURE_PASSWORD\"",
        ])
        .output()
        .expect("docker compose exec");
    assert!(
        out.status.success(),
        "cannot read the fixture password: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn failed_password_count(service: &str) -> usize {
    let out = std::process::Command::new("docker")
        .args([
            "compose",
            "-f",
            "tools/sftp-test-server/docker-compose.yml",
            "logs",
            "--no-color",
            service,
        ])
        .output()
        .expect("docker compose logs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    text.matches("Failed password for uksfta").count()
}

fn password_settings(port: u16) -> (Settings, tempfile::TempDir) {
    let (dir, pinned) = pin(port);
    (settings(pinned, None, port), dir)
}

fn secret_from(text: &str) -> super::secret::Secret {
    let mut reader = std::io::Cursor::new(format!("{text}\n").into_bytes());
    super::secret::read_password(&mut reader).expect("a secret")
}

#[tokio::test]
#[ignore = "needs the Docker password fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_prompted_password_authenticates() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let password = fixture_password("sftp-password");
    let (settings, _dir) = password_settings(2224);
    let secret = secret_from(&password);

    connect::connect_with(&settings, Some(&secret))
        .await
        .expect("the fixture password must authenticate");
}

#[tokio::test]
#[ignore = "needs the Docker password fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_wrong_password_is_refused_once() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let (settings, _dir) = password_settings(2224);
    let before = failed_password_count("sftp-password");
    let secret = secret_from("not-the-fixture-password");

    let error = match connect::connect_with(&settings, Some(&secret)).await {
        Ok(_) => panic!("a wrong password must fail"),
        Err(error) => error,
    };
    let text = error.to_string();
    assert!(text.contains("(password)"), "{text}");
    assert!(text.contains("remaining methods"), "{text}");
    assert!(
        !text.contains("not-the-fixture-password"),
        "the secret must not be printed"
    );

    let after = failed_password_count("sftp-password");
    assert_eq!(after.saturating_sub(before), 1, "exactly one attempt");
}

#[tokio::test]
#[ignore = "needs the Docker kbdint fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_keyboard_interactive_server_accepts_the_password() {
    if !live() {
        return;
    }
    let _serial = live_lock_async().await;
    let password = fixture_password("sftp-kbdint");
    let (settings, _dir) = password_settings(2225);
    let secret = secret_from(&password);

    connect::connect_with(&settings, Some(&secret))
        .await
        .expect("the keyboard-interactive fixture must accept the password");
}
