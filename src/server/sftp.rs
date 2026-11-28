//! Thin wrapper around the `sftp` client.
//!
//! Commands travel on standard input. `sftp -b` puts the client in
//! non-interactive mode and suppresses the password prompt, so the prompt
//! and askpass paths stay available here. One run opens one multiplexed
//! connection, so a prompt appears at most once per run.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use super::config::{Settings, Transfer};
use super::hostkey::{self, HostKeyPolicy};
use crate::error::UksftaError;
use crate::util::sanitize_for_terminal;

/// Environment override for the client binary. A test points this at a stub.
const BINARY_ENV: &str = "UKSFTA_SFTP_BIN";
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// Keep the connection alive, and fail a dead link in seconds.
const LIVENESS: [&str; 3] = [
    "ServerAliveInterval=15",
    "ServerAliveCountMax=3",
    "ConnectTimeout=15",
];

/// Multiplexing. The master holds the connection, and every child names the
/// socket so it rides that connection rather than opening its own. The
/// persist window is the backstop for a run that is killed.
#[cfg(unix)]
const CONTROL_PERSIST: &str = "ControlPersist=60";

/// The longest control socket path that is safe to pass. `sun_path` holds 108
/// bytes with its terminator, and a longer path fails at connect time.
#[cfg(unix)]
const MAX_SOCKET_PATH: usize = 100;

/// The master binary. It ships with the client.
#[cfg(unix)]
const SSH: &str = "ssh";

/// The run's control socket. One process is one run, so every child in the
/// upload and verification phases shares it.
#[cfg(unix)]
struct ControlSocket {
    path: PathBuf,
    destination: String,
    /// Held while the first child of the run starts the master, so the
    /// children that follow wait for it instead of racing it.
    started: Mutex<bool>,
}

#[cfg(unix)]
impl ControlSocket {
    fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Stop the master and remove the socket. Cleanup never fails a command
    /// and never waits long: the persist window is the backstop.
    fn stop(&self) {
        let live = *self.started.lock().unwrap_or_else(PoisonError::into_inner);
        if live {
            self.ask_master_to_exit();
        }
        let _ = fs::remove_file(&self.path);
    }

    fn ask_master_to_exit(&self) {
        let Ok(mut child) = Command::new(SSH)
            .args([
                "-O".to_string(),
                "exit".to_string(),
                "-o".to_string(),
                format!("ControlPath={}", self.path.display()),
                self.destination.clone(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return;
        };
        for _ in 0..CLEANUP_POLLS {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(CLEANUP_POLL),
                Err(_) => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// A master that has not answered the exit request in a second is left to
/// its persist window.
#[cfg(unix)]
const CLEANUP_POLL: std::time::Duration = std::time::Duration::from_millis(20);
#[cfg(unix)]
const CLEANUP_POLLS: usize = 50;

/// Stop the master, then remove the socket. A socket that has already gone
/// is not an error.
#[cfg(unix)]
impl Drop for ControlSocket {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The options that make one connection the master, or that make a child ride
/// it. `sftp` forces `ControlMaster=no` on its own child, so a child can never
/// become the master: the run starts one with `ssh -N` and the children point
/// at its socket.
#[cfg(unix)]
fn control_options(path: &std::path::Path, master: bool) -> Vec<String> {
    let role = if master {
        "ControlMaster=yes"
    } else {
        "ControlMaster=auto"
    };
    vec![
        "-o".to_string(),
        role.to_string(),
        "-o".to_string(),
        CONTROL_PERSIST.to_string(),
        "-o".to_string(),
        format!("ControlPath={}", path.display()),
    ]
}

/// The control handle for one child. Windows has no multiplexing, so the
/// handle holds nothing there.
#[cfg(unix)]
type Control = Arc<ControlSocket>;
#[cfg(not(unix))]
type Control = ();

/// The client. It holds no credentials: `sftp` reads the key, the agent or
/// the terminal prompt itself.
pub struct Sftp {
    binary: String,
    destination: String,
    port: u16,
    identity: Option<PathBuf>,
    transfer: Transfer,
    known_hosts: PathBuf,
    policy: HostKeyPolicy,
    /// Refuse to prompt. The probe must never ask for a second password.
    batch_mode: bool,
    /// The run's multiplexing socket, when the platform and the mode allow.
    control: Option<Control>,
}

impl Sftp {
    /// A client for a phase that runs alone: it starts or rides the run's
    /// shared connection. A parallel upload worker must use `own_connection`
    /// instead, because one TCP stream per worker is what gives the upload
    /// its throughput on a link with latency.
    pub fn new(settings: &Settings) -> Self {
        let binary = std::env::var(BINARY_ENV).unwrap_or_else(|_| "sftp".to_string());
        Self {
            binary,
            destination: settings.destination.clone(),
            port: settings.port,
            identity: settings.identity.clone(),
            transfer: settings.transfer,
            known_hosts: settings.known_hosts.clone(),
            policy: settings.host_key_policy,
            batch_mode: false,
            #[cfg(unix)]
            control: Some(run_socket(&settings.destination)),
            #[cfg(not(unix))]
            control: None,
        }
    }

    /// A client that opens its own connection, for a phase that runs beside
    /// its peers. The run's shared connection is left alone, so a group of
    /// workers keeps one TCP stream each.
    pub fn own_connection(settings: &Settings) -> Self {
        #[cfg(unix)]
        {
            let mut client = Self::new(settings);
            client.control = None;
            client
        }
        #[cfg(not(unix))]
        {
            Self::new(settings)
        }
    }

    /// A client that never prompts, for the link probe. It times one
    /// connection to one stream, so a shared master would hide the handshake
    /// it measures.
    pub fn probe(settings: &Settings) -> Self {
        let mut client = Self::own_connection(settings);
        client.batch_mode = true;
        client
    }

    /// The full argument list for one client run. `LogLevel=ERROR` keeps
    /// ssh warnings out of a successful run, and still shows the host key
    /// error that a failure needs. It does not suppress the meter.
    fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        args.push("-P".to_string());
        args.push(self.port.to_string());
        args.extend([
            "-R".to_string(),
            self.transfer.requests.to_string(),
            "-B".to_string(),
            self.transfer.buffer.to_string(),
        ]);
        if let Some(rate) = self.transfer.limit {
            args.push("-l".to_string());
            args.push(limit_kbit(rate).to_string());
        }
        args.extend(self.link_options());
        #[cfg(unix)]
        if let Some(control) = &self.control {
            args.extend(control_options(control.path(), false));
        }
        #[cfg(not(unix))]
        debug_assert!(
            self.control.is_none(),
            "windows has no control socket to pass"
        );
        args.push(self.destination.clone());
        args
    }

    /// The options that set up the link. The master uses the same set, so the
    /// identity, the liveness window and the pinned host key hold for both.
    fn link_options(&self) -> Vec<String> {
        let mut options = Vec::new();
        if let Some(path) = &self.identity {
            options.push("-i".to_string());
            options.push(path.display().to_string());
        }
        for option in LIVENESS {
            options.push("-o".to_string());
            options.push(option.to_string());
        }
        options.extend(["-o".to_string(), "LogLevel=ERROR".to_string()]);
        if self.batch_mode {
            options.extend(["-o".to_string(), "BatchMode=yes".to_string()]);
        }
        options.extend([
            "-o".to_string(),
            format!("StrictHostKeyChecking={}", self.policy.as_str()),
            "-o".to_string(),
            format!("UserKnownHostsFile={}", self.known_hosts.display()),
            "-o".to_string(),
            "UpdateHostKeys=no".to_string(),
        ]);
        options
    }

    /// The master's argument list. It carries the same link options and no
    /// transfer option, because it moves no file.
    #[cfg(unix)]
    fn master_args(&self) -> Vec<String> {
        let mut args = vec![
            "-N".to_string(),
            "-f".to_string(),
            "-p".to_string(),
            self.port.to_string(),
        ];
        args.extend(self.link_options());
        if let Some(control) = &self.control {
            args.extend(control_options(control.path(), true));
        }
        args.push(self.destination.clone());
        args
    }

    /// Start the run's master, once. A failure is silent: the children then
    /// open their own connections, which is the behaviour without a socket.
    #[cfg(unix)]
    fn ensure_master(&self) {
        let Some(control) = &self.control else {
            return;
        };
        if std::env::var_os(BINARY_ENV).is_some() {
            return;
        }
        let mut started = control
            .started
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if *started {
            return;
        }
        *started = true;
        let mut command = Command::new(SSH);
        command.args(self.master_args());
        // A passphrase or password prompt still reaches the terminal; the
        // link itself writes nothing the caller needs.
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let _ = command.status();
    }

    /// A client run with its commands on standard input.
    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command.args(self.args()).stdin(Stdio::piped());
        command
    }

    fn spawn(&self, mut command: Command) -> Result<Child, UksftaError> {
        command
            .spawn()
            .map_err(|e| UksftaError::Input(format!("cannot run {}: {e}", self.binary)))
    }

    /// Write the commands, then close standard input so the client sees the
    /// end of the batch.
    fn send(&self, child: &mut Child, lines: &[String]) -> Result<(), UksftaError> {
        let mut body = String::new();
        for line in lines {
            body.push_str(line);
            body.push('\n');
        }
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(body.as_bytes()) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(UksftaError::Input(format!(
                    "cannot send commands to {}: {e}",
                    self.binary
                )));
            }
        }
        Ok(())
    }

    /// Send one phase of commands and return the client's standard output.
    ///
    /// # Errors
    /// Returns `Input` when the client cannot start, cannot take the
    /// commands, or exits non-zero.
    pub fn batch(&self, lines: &[String]) -> Result<String, UksftaError> {
        self.batch_tracked(lines, |_| {})
    }

    /// Send one phase of commands, passing each output line to `on_line` as
    /// it arrives. The whole output is still returned for the caller.
    ///
    /// A long phase uses this to report progress while the client runs, so
    /// one child can cover many files.
    ///
    /// # Errors
    /// Returns `Input` when the client cannot start, cannot take the
    /// commands, cannot be read, or exits non-zero.
    pub fn batch_tracked(
        &self,
        lines: &[String],
        mut on_line: impl FnMut(&str),
    ) -> Result<String, UksftaError> {
        if lines.is_empty() {
            return Ok(String::new());
        }

        #[cfg(unix)]
        self.ensure_master();
        let mut command = self.command();
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = self.spawn(command)?;
        self.send(&mut child, lines)?;

        // Standard error is drained on its own thread. A chatty failure must
        // not fill the pipe while the caller reads standard output.
        let stderr = child.stderr.take();
        let drain = std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut stderr) = stderr {
                let _ = stderr.read_to_string(&mut text);
            }
            text
        });

        let mut stdout = String::new();
        let reader = child.stdout.take().ok_or_else(|| {
            UksftaError::Input(format!("cannot read the output of {}", self.binary))
        })?;
        for line in BufReader::new(reader).lines() {
            let line = line.map_err(|e| {
                UksftaError::Input(format!("cannot read the output of {}: {e}", self.binary))
            })?;
            on_line(&line);
            stdout.push_str(&line);
            stdout.push('\n');
        }

        let status = child.wait().map_err(|e| {
            UksftaError::Input(format!("cannot read the output of {}: {e}", self.binary))
        })?;
        let stderr = drain.join().unwrap_or_default();

        if !status.success() {
            if let Some(text) =
                hostkey::describe_failure(&stderr, &self.destination, &self.known_hosts)
            {
                return Err(UksftaError::Input(text));
            }
            let detail = last_line(&stderr)
                .or_else(|| last_line(&stdout))
                .unwrap_or_else(|| "no output".to_string());
            return Err(UksftaError::Input(format!("sftp failed: {detail}")));
        }
        Ok(stdout)
    }
}

/// The `-l` value. The sftp man page counts Kbit/s, so bytes become
/// kilobits rounded up.
pub fn limit_kbit(bytes_per_second: u64) -> u64 {
    let kbit = (u128::from(bytes_per_second) * 8).div_ceil(1000);
    u64::try_from(kbit).unwrap_or(u64::MAX)
}

/// The last non-empty line of client output, stripped of terminal escapes.
fn last_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(|line| sanitize_for_terminal(line).trim().to_string())
}

/// Quote one client argument. A quote or control character in a remote path
/// could start a new command, so it is refused instead.
///
/// # Errors
/// Returns `Input` for an empty value or one that a command cannot hold.
pub fn quote(value: &str) -> Result<String, UksftaError> {
    let unsafe_char =
        value.is_empty() || value.contains('"') || value.chars().any(char::is_control);
    if unsafe_char {
        return Err(UksftaError::Input(format!(
            "refusing to send an unsafe path: {value:?}"
        )));
    }
    Ok(format!("\"{value}\""))
}

/// A private scratch directory that removes itself, for downloaded files.
pub struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    pub fn new(prefix: &str) -> Result<Self, UksftaError> {
        let path = unique_path(prefix);
        fs::create_dir_all(&path).map_err(UksftaError::Io)?;
        Ok(Self { path })
    }

    pub fn join(&self, relative: &str) -> PathBuf {
        self.path.join(relative)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn unique_path(prefix: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{prefix}{}-{id}", std::process::id()))
}

/// The socket of this run. A fresh path per process keeps two concurrent runs
/// apart, and keeps a socket left by a killed run from blocking the next one.
#[cfg(unix)]
fn run_socket(destination: &str) -> Arc<ControlSocket> {
    static LIVE: OnceLock<Mutex<Weak<ControlSocket>>> = OnceLock::new();
    static NEXT_RUN: AtomicU64 = AtomicU64::new(0);

    let slot = LIVE.get_or_init(|| Mutex::new(Weak::new()));
    let mut live = slot.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(socket) = live.upgrade() {
        if socket.destination == destination {
            return socket;
        }
    }
    let run = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
    let socket = Arc::new(ControlSocket {
        path: socket_path(run),
        destination: destination.to_string(),
        started: Mutex::new(false),
    });
    *live = Arc::downgrade(&socket);
    socket
}

/// The control socket path: a short runtime directory, a fixed prefix, the
/// process id and a run counter. A path over the socket limit loses the
/// runtime directory first.
#[cfg(unix)]
fn socket_path(run: u64) -> PathBuf {
    let name = format!("uksfta-cm-{}-{run}", std::process::id());
    let fallback = PathBuf::from("/tmp");
    for dir in runtime_dirs() {
        let candidate = dir.join(&name);
        if candidate.as_os_str().len() <= MAX_SOCKET_PATH {
            return candidate;
        }
    }
    fallback.join(name)
}

/// The directories that may hold a socket, most private first.
#[cfg(unix)]
fn runtime_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .into_iter()
        .collect();
    dirs.push(std::env::temp_dir());
    dirs.retain(|dir| dir.is_dir());
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(transfer: Transfer) -> Sftp {
        Sftp {
            binary: "sftp".to_string(),
            destination: "tester@example.invalid".to_string(),
            port: 2022,
            identity: Some(PathBuf::from("/home/user/.ssh/id_ed25519")),
            transfer,
            known_hosts: PathBuf::from("/home/user/.ssh/known_hosts"),
            policy: HostKeyPolicy::Yes,
            batch_mode: false,
            control: None,
        }
    }

    /// The control path of a client, when it has one.
    #[cfg(unix)]
    fn control_path(client: &Sftp) -> Option<String> {
        client
            .args()
            .windows(2)
            .find(|pair| pair[0] == "-o" && pair[1].starts_with("ControlPath="))
            .map(|pair| pair[1].clone())
    }

    /// Settings that reach a real client constructor.
    fn settings() -> Settings {
        Settings {
            destination: "tester@example.invalid".to_string(),
            port: 2022,
            root: "/srv".to_string(),
            identity: Some(PathBuf::from("/home/user/.ssh/id_ed25519")),
            transfer: Transfer::default(),
            known_hosts: PathBuf::from("/home/user/.ssh/known_hosts"),
            host_key_policy: HostKeyPolicy::Yes,
            launch_line: None,
            server_cfg: None,
            segment: crate::server::config::Segment::default(),
        }
    }

    #[test]
    fn the_client_arguments_stay_inside_the_buffer_limit() {
        let client = client(Transfer::default());

        let args = client.args();
        assert!(
            !args.iter().any(|arg| arg == "262144"),
            "the buffer must stay inside the server-safe limit: {args:?}"
        );
        let buffer = args
            .iter()
            .position(|arg| arg == "-B")
            .map(|i| args[i + 1].clone());
        assert_eq!(buffer.as_deref(), Some("32768"));
        for option in LIVENESS {
            assert!(
                args.windows(2).any(|pair| pair == ["-o", option]),
                "{args:?}"
            );
        }
        assert_eq!(
            args.last().map(String::as_str),
            Some("tester@example.invalid")
        );
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-i", "/home/user/.ssh/id_ed25519"]));
    }

    #[test]
    fn a_limit_becomes_the_client_kbit_argument() {
        let transfer = Transfer {
            limit: Some(5 * 1024 * 1024),
            ..Transfer::default()
        };
        let args = client(transfer).args();
        let value = args
            .iter()
            .position(|arg| arg == "-l")
            .map(|index| args[index + 1].as_str());
        assert_eq!(value, Some("41944"));
        assert_eq!(limit_kbit(512_000), 4096);
        assert_eq!(limit_kbit(1), 1, "the value rounds up");
        assert!(!client(Transfer::default())
            .args()
            .iter()
            .any(|arg| arg == "-l"));
    }

    #[test]
    fn the_error_log_level_replaces_quiet_on_every_run() {
        let args = client(Transfer::default()).args();

        assert!(
            !args.iter().any(|arg| arg == "-q"),
            "no run may pass -q: {args:?}"
        );
        assert!(
            args.windows(2).any(|pair| pair == ["-o", "LogLevel=ERROR"]),
            "{args:?}"
        );
        let level = args.iter().position(|arg| arg == "LogLevel=ERROR");
        let strict = args
            .iter()
            .position(|arg| arg == "StrictHostKeyChecking=yes");
        assert!(
            level < strict,
            "the log level comes before the host key options: {args:?}"
        );
        assert!(
            args.windows(2).any(|pair| pair == ["-B", "32768"]),
            "{args:?}"
        );
        for option in LIVENESS {
            assert!(
                args.windows(2).any(|pair| pair == ["-o", option]),
                "{args:?}"
            );
        }
    }

    #[test]
    fn the_default_transfer_values_are_the_baseline() {
        let args = client(Transfer::default()).args();

        assert!(args.windows(2).any(|pair| pair == ["-R", "64"]), "{args:?}");
        assert!(
            args.windows(2).any(|pair| pair == ["-B", "32768"]),
            "{args:?}"
        );
    }

    #[test]
    fn the_configured_transfer_values_reach_the_arguments() {
        let args = client(Transfer {
            buffer: 261_120,
            requests: 128,
            ..Transfer::default()
        })
        .args();

        assert!(
            args.windows(2).any(|pair| pair == ["-B", "261120"]),
            "{args:?}"
        );
        assert!(
            args.windows(2).any(|pair| pair == ["-R", "128"]),
            "{args:?}"
        );
        let requests = args.iter().position(|arg| arg == "-R");
        let buffer = args.iter().position(|arg| arg == "-B");
        assert!(requests < buffer, "-R must come before -B: {args:?}");
    }

    #[test]
    fn the_host_key_options_are_pinned_for_both_policies() {
        for (policy, expected) in [
            (HostKeyPolicy::Yes, "StrictHostKeyChecking=yes"),
            (HostKeyPolicy::AcceptNew, "StrictHostKeyChecking=accept-new"),
        ] {
            let mut pinned = client(Transfer::default());
            pinned.policy = policy;
            let args = pinned.args();

            assert!(
                args.windows(2).any(|pair| pair == ["-o", expected]),
                "{args:?}"
            );
            assert!(
                args.windows(2)
                    .any(|pair| pair == ["-o", "UserKnownHostsFile=/home/user/.ssh/known_hosts"]),
                "{args:?}"
            );
            assert!(
                args.windows(2)
                    .any(|pair| pair == ["-o", "UpdateHostKeys=no"]),
                "{args:?}"
            );
            assert_eq!(
                args.last().map(String::as_str),
                Some("tester@example.invalid"),
                "the destination stays last: {args:?}"
            );
            assert!(
                !args
                    .iter()
                    .any(|arg| arg.contains("BatchMode") || arg == "StrictHostKeyChecking=no"),
                "{args:?}"
            );
        }
    }

    #[test]
    fn quote_wraps_a_plain_path() {
        assert_eq!(quote("/a b/c").unwrap(), "\"/a b/c\"");
    }

    #[test]
    fn quote_refuses_command_injection() {
        assert!(quote("/a\"\nb").is_err());
        assert!(quote("").is_err());
        assert!(quote("/a\ncd /").is_err());
    }

    #[test]
    fn scratch_dir_removes_itself() {
        let dir = ScratchDir::new("uksfta-test-scratch-").unwrap();
        let path = dir.join("x");
        fs::write(&path, b"x").unwrap();
        assert!(path.exists());
        drop(dir);
        assert!(!path.exists());
    }

    #[test]
    fn last_line_skips_blank_and_keeps_text() {
        assert_eq!(last_line("a\n\nb\n").as_deref(), Some("b"));
        assert_eq!(last_line("   \n"), None);
    }

    #[cfg(unix)]
    #[test]
    fn one_run_multiplexes_its_children_over_one_socket() {
        let first = Sftp {
            control: Some(run_socket("tester@example.invalid")),
            ..client(Transfer::default())
        };
        let second = Sftp {
            control: Some(run_socket("tester@example.invalid")),
            ..client(Transfer::default())
        };

        let path = control_path(&first).expect("a control path on unix");
        assert_eq!(
            Some(path.clone()),
            control_path(&second),
            "every child of a run rides the same socket"
        );
        let socket = path.trim_start_matches("ControlPath=");
        assert!(
            socket.contains(&std::process::id().to_string()),
            "the path names the run: {socket}"
        );
        assert!(
            !socket.contains("tester@example.invalid"),
            "the path must not carry the destination: {socket}"
        );
        assert!(
            socket.len() <= MAX_SOCKET_PATH,
            "the socket path stays short: {socket}"
        );

        let args = first.args();
        for pair in [
            ["-o", "ControlMaster=auto"],
            ["-o", CONTROL_PERSIST],
            ["-o", path.as_str()],
            ["-o", "LogLevel=ERROR"],
            ["-o", "StrictHostKeyChecking=yes"],
            ["-o", "UpdateHostKeys=no"],
            ["-i", "/home/user/.ssh/id_ed25519"],
            ["-B", "32768"],
        ] {
            assert!(
                args.windows(2).any(|seen| seen == pair),
                "{pair:?} is missing"
            );
        }
        assert_eq!(
            args.last().map(String::as_str),
            Some("tester@example.invalid"),
            "the destination stays last: {args:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_master_takes_the_same_link_options_and_no_transfer_option() {
        let client = Sftp {
            control: Some(run_socket("tester@example.invalid")),
            ..client(Transfer::default())
        };
        let path = control_path(&client).expect("a control path on unix");
        let master = client.master_args();

        for flag in ["-N", "-f"] {
            assert!(master.iter().any(|arg| arg == flag), "{flag} is missing");
        }
        for pair in [
            ["-p", "2022"],
            ["-o", "ControlMaster=yes"],
            ["-o", CONTROL_PERSIST],
            ["-o", path.as_str()],
            ["-o", "StrictHostKeyChecking=yes"],
            ["-i", "/home/user/.ssh/id_ed25519"],
            ["-o", "UserKnownHostsFile=/home/user/.ssh/known_hosts"],
        ] {
            assert!(
                master.windows(2).any(|seen| seen == pair),
                "{pair:?} is missing: {master:?}"
            );
        }
        assert!(
            !master
                .iter()
                .any(|arg| arg == "-R" || arg == "-B" || arg == "-l" || arg == "-P"),
            "the master moves no file: {master:?}"
        );
        assert_eq!(
            master.last().map(String::as_str),
            Some("tester@example.invalid"),
            "the destination stays last: {master:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_parallel_worker_opens_its_own_connection() {
        let alone = Sftp::new(&settings());
        assert!(control_path(&alone).is_some(), "a lone phase shares");

        let worker = Sftp::own_connection(&settings());
        assert!(
            control_path(&worker).is_none(),
            "a worker beside its peers keeps a stream of its own"
        );
        for pair in [
            ["-o", "StrictHostKeyChecking=yes"],
            ["-o", "UpdateHostKeys=no"],
            ["-i", "/home/user/.ssh/id_ed25519"],
        ] {
            assert!(
                worker.args().windows(2).any(|seen| seen == pair),
                "{pair:?} is missing"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_probe_carries_no_socket() {
        let run = Sftp::new(&settings());
        assert!(control_path(&run).is_some(), "a run has a socket");
        let probe = Sftp::probe(&settings());
        assert!(
            control_path(&probe).is_none(),
            "the probe measures one link, so it opens its own"
        );
        assert!(probe
            .args()
            .windows(2)
            .any(|pair| pair == ["-o", "BatchMode=yes"]));
    }

    #[cfg(unix)]
    #[test]
    fn a_control_socket_removes_its_file_and_tolerates_its_absence() {
        let path = std::env::temp_dir().join(format!("uksfta-test-cm-{}", std::process::id()));
        fs::write(&path, b"x").unwrap();
        drop(ControlSocket {
            path: path.clone(),
            destination: "tester@example.invalid".to_string(),
            started: Mutex::new(false),
        });
        assert!(!path.exists());
        drop(ControlSocket {
            path,
            destination: "tester@example.invalid".to_string(),
            started: Mutex::new(false),
        });
    }

    #[cfg(not(unix))]
    #[test]
    fn windows_passes_no_multiplexing_options() {
        let args = Sftp::new(&settings()).args();
        assert!(
            !args.iter().any(|arg| arg.contains("ControlMaster")
                || arg.contains("ControlPersist")
                || arg.contains("ControlPath")),
            "{args:?}"
        );
    }
}
