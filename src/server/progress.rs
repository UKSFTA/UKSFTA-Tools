//! The whole-upload progress view: one total, one row per planned file, and
//! the per-connection rows that the segmented file expands into.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use serde::Serialize;

use super::report::UploadUnit;
use crate::error::UksftaError;
use crate::util::human_size;

/// How often the plain writer prints one line.
const PLAIN_INTERVAL: Duration = Duration::from_secs(5);
/// How often one `progress` event reaches standard error.
const JSON_INTERVAL: Duration = Duration::from_secs(1);

/// Where the progress output goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// No progress output.
    Off,
    /// The whole-upload view on the terminal.
    Terminal,
    /// One plain line every five seconds.
    Plain,
    /// One JSON object per line on standard error.
    Json,
}

impl Mode {
    pub fn is_on(self) -> bool {
        !matches!(self, Mode::Off)
    }
}

/// Pick the mode from the flags and the terminal.
pub fn resolve(requested: bool, progress_json: bool, stderr_is_terminal: bool) -> Mode {
    if progress_json {
        Mode::Json
    } else if !requested {
        Mode::Off
    } else if stderr_is_terminal {
        Mode::Terminal
    } else {
        Mode::Plain
    }
}

/// The fallback line for a segmented failure that the whole-file path takes.
pub fn fallback_line(error: &UksftaError) -> String {
    let kind = match error {
        UksftaError::Input(_) => "Input",
        UksftaError::Parse(_) => "Parse",
        UksftaError::Check(_) => "Check",
        UksftaError::Auth(_) => "Auth",
        UksftaError::Locked(_) => "Locked",
        UksftaError::Io(_) => "Io",
    };
    format!("segment: not used: {kind}: {error}; using the whole-file path")
}

/// The whole percentage, rounded to the nearest point.
fn percent_of(done: u64, total: u64) -> u64 {
    (done.saturating_mul(100).saturating_add(total / 2))
        .checked_div(total)
        .map_or(100, |value| value.min(100))
}

/// One plain line for a non-terminal stream.
pub fn plain_line(done: u64, total: u64, elapsed: Duration, eta: Option<Duration>) -> String {
    let percent = percent_of(done, total);
    let seconds = elapsed.as_secs_f64().max(f64::EPSILON);
    let rate = done as f64 / seconds / 1_000_000.0;
    let eta = eta.map_or_else(|| "--:--".to_string(), clock);
    format!(
        "progress {percent}% {:.1}/{:.1} MB {rate:.1} MB/s elapsed {} ETA {eta}",
        done as f64 / 1_000_000.0,
        total as f64 / 1_000_000.0,
        clock(elapsed)
    )
}

fn clock(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// The resume marker printed before a resumed segmented transfer.
pub fn resume_line(from: u64, total: u64) -> String {
    format!(
        "segment: resuming at {}% ({from} of {total} bytes)",
        percent_of(from, total)
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Waiting,
    Uploading,
    Done,
}

impl State {
    fn label(self) -> &'static str {
        match self {
            State::Waiting => "waiting",
            State::Uploading => "uploading",
            State::Done => "done",
        }
    }
}

struct Row {
    remote: String,
    name: String,
    size: u64,
    done: u64,
    state: State,
}

struct Shared {
    rows: Vec<Row>,
    total: u64,
    done: u64,
    started: Instant,
}

impl Shared {
    fn active(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.state == State::Uploading)
            .count()
    }

    fn eta(&self) -> Option<Duration> {
        let elapsed = self.started.elapsed().as_secs_f64();
        if self.done == 0 || elapsed <= f64::EPSILON {
            return None;
        }
        let rate = self.done as f64 / elapsed;
        let left = self.total.saturating_sub(self.done) as f64 / rate;
        Some(Duration::from_secs_f64(left.max(0.0)))
    }
}

#[derive(Serialize)]
struct StartEvent {
    schema_version: u8,
    event: &'static str,
    files: usize,
    bytes: u64,
}

#[derive(Serialize)]
struct ProgressEvent {
    schema_version: u8,
    event: &'static str,
    file: String,
    bytes: u64,
    total: u64,
    rate: f64,
    eta_seconds: u64,
    active: usize,
}

#[derive(Serialize)]
struct FileDoneEvent {
    schema_version: u8,
    event: &'static str,
    file: String,
    bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
}

#[derive(Serialize)]
struct ResumeEvent {
    schema_version: u8,
    event: &'static str,
    file: String,
    bytes: u64,
    total: u64,
}

#[derive(Serialize)]
struct ConnectionsEvent {
    schema_version: u8,
    event: &'static str,
    from: u8,
    to: u8,
}

#[derive(Serialize)]
struct DoneEvent {
    schema_version: u8,
    event: &'static str,
    bytes: u64,
    seconds: f64,
    rate: f64,
}

/// The segmented file's link to the view.
#[derive(Clone)]
pub struct SegmentedTrace {
    pub progress: Arc<UploadProgress>,
    pub remote: String,
}

/// Start one range on a connection bar.
pub fn range_started(bars: &[ProgressBar], worker: usize, index: u32, len: u32) {
    if let Some(bar) = bars.get(worker) {
        bar.set_length(u64::from(len));
        bar.set_position(0);
        bar.set_message(format!("range {}", index + 1));
    }
}

/// Count bytes on a connection bar.
pub fn range_advanced(bars: &[ProgressBar], worker: usize, bytes: u64) {
    if let Some(bar) = bars.get(worker) {
        bar.inc(bytes);
    }
}

/// A range finished; leave its bar at 100% until the next range.
pub fn range_finished(bars: &[ProgressBar], worker: usize) {
    if let Some(bar) = bars.get(worker) {
        bar.set_position(bar.length().unwrap_or(0));
        bar.set_message("done");
    }
}

/// A range failed and goes back to the queue.
pub fn range_retried(bars: &[ProgressBar], worker: usize) {
    if let Some(bar) = bars.get(worker) {
        bar.set_position(0);
        bar.set_message("retry");
    }
}

/// The template of a working connection row.
const CONNECTION_STYLE: &str = "  {spinner} conn {prefix:>2} {bar:24} {bytes:>9}/{total_bytes:<9} \
                               {bytes_per_sec:>9} ETA {eta_precise} {msg}";
/// The template of a connection row above the active limit.
const IDLE_STYLE: &str = "  conn {prefix:>2} {wide_msg}";

fn connection_style(template: &str) -> ProgressStyle {
    ProgressStyle::with_template(template)
        .expect("the connection progress template is valid")
        .progress_chars("=>-")
}

/// Show or hide one connection row. A hidden row keeps its bar, so a worker
/// that still holds it cannot panic.
pub fn connection_active(bars: &[ProgressBar], worker: usize, active: bool) {
    let Some(bar) = bars.get(worker) else {
        return;
    };
    if active {
        bar.set_style(connection_style(CONNECTION_STYLE));
        bar.set_position(0);
        bar.set_message("waiting");
    } else {
        bar.set_style(connection_style(IDLE_STYLE));
        bar.set_position(0);
        bar.set_message("idle");
    }
}

/// Remove the connection bars once the segmented file is finished.
pub fn clear_bars(bars: &[ProgressBar]) {
    for bar in bars {
        bar.finish_and_clear();
    }
}

/// The whole-upload view shared by every sender in one run.
pub struct UploadProgress {
    mode: Mode,
    shared: Mutex<Shared>,
    multi: Option<MultiProgress>,
    total_bar: Option<ProgressBar>,
    rows: Vec<ProgressBar>,
    last_json: Mutex<Instant>,
    stop: AtomicBool,
    finished: AtomicBool,
    ticker: Mutex<Option<JoinHandle<()>>>,
}

impl UploadProgress {
    /// Build the view. Nothing is drawn for [`Mode::Off`].
    pub fn new(units: &[UploadUnit], mode: Mode) -> Arc<Self> {
        let rows: Vec<Row> = units
            .iter()
            .map(|unit| Row {
                remote: unit.remote.clone(),
                name: name_of(&unit.remote),
                size: unit.size,
                done: 0,
                state: State::Waiting,
            })
            .collect();
        let total = rows.iter().map(|row| row.size).sum();

        let (multi, total_bar, bars) = if mode == Mode::Terminal {
            let multi = MultiProgress::with_draw_target(ProgressDrawTarget::stderr());
            let total_bar = multi.add(ProgressBar::new(total));
            total_bar.set_style(
                ProgressStyle::with_template(
                    "{spinner} total {bar:32} {percent:>3}% {bytes:>12}/{total_bytes:<12} \
                     {bytes_per_sec:>11} elapsed {elapsed_precise} ETA {eta_precise}",
                )
                .expect("the total progress template is valid")
                .progress_chars("=>-"),
            );
            total_bar.enable_steady_tick(Duration::from_millis(250));
            let bars = rows
                .iter()
                .map(|row| {
                    let bar = multi.add(ProgressBar::new(row.size));
                    bar.set_style(
                        ProgressStyle::with_template(
                            "{prefix:>9} {wide_msg:<44} {bar:20} {percent:>3}% {eta_precise}",
                        )
                        .expect("the file progress template is valid")
                        .progress_chars("=>-"),
                    );
                    bar.set_prefix(row.state.label());
                    bar.set_message(format!("{}  {}", row.name, human_size(row.size)));
                    // A ticker starts only when the row begins to upload, so
                    // a 114-file pack does not run 114 of them at once.
                    bar
                })
                .collect();
            (Some(multi), Some(total_bar), bars)
        } else {
            (None, None, Vec::new())
        };

        let view = Arc::new(Self {
            mode,
            shared: Mutex::new(Shared {
                rows,
                total,
                done: 0,
                started: Instant::now(),
            }),
            multi,
            total_bar,
            rows: bars,
            last_json: Mutex::new(Instant::now() - JSON_INTERVAL),
            stop: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            ticker: Mutex::new(None),
        });

        match mode {
            Mode::Json => view.emit_start(),
            Mode::Plain => view.start_plain_ticker(),
            Mode::Off | Mode::Terminal => {}
        }
        view
    }

    fn emit_start(&self) {
        let shared = self
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let event = StartEvent {
            schema_version: 1,
            event: "start",
            files: shared.rows.len(),
            bytes: shared.total,
        };
        emit(&event);
    }

    fn start_plain_ticker(self: &Arc<Self>) {
        let view = Arc::clone(self);
        let handle = std::thread::spawn(move || {
            let mut printed = Instant::now();
            while !view.stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(250));
                if printed.elapsed() >= PLAIN_INTERVAL {
                    printed = Instant::now();
                    let (done, total, elapsed, eta) = {
                        let shared = view
                            .shared
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        (
                            shared.done,
                            shared.total,
                            shared.started.elapsed(),
                            shared.eta(),
                        )
                    };
                    eprintln!("{}", plain_line(done, total, elapsed, eta));
                }
            }
        });
        *self
            .ticker
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(handle);
    }

    fn index(&self, remote: &str) -> Option<usize> {
        self.shared
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .rows
            .iter()
            .position(|row| row.remote == remote)
    }

    /// Hide the view while a password prompt owns the terminal.
    pub fn hide(&self) {
        if let Some(multi) = &self.multi {
            multi.set_draw_target(ProgressDrawTarget::hidden());
        }
    }

    /// Show the view again after the prompt.
    pub fn show(&self) {
        if let Some(multi) = &self.multi {
            multi.set_draw_target(ProgressDrawTarget::stderr());
        }
    }

    /// Mark one file as sending.
    pub fn file_started(&self, remote: &str) {
        let Some(index) = self.index(remote) else {
            return;
        };
        {
            let mut shared = self
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            shared.rows[index].state = State::Uploading;
        }
        if let Some(bar) = self.rows.get(index) {
            bar.set_prefix(State::Uploading.label());
            bar.enable_steady_tick(Duration::from_millis(250));
        }
    }

    /// Count acknowledged bytes for one file.
    pub fn advance(&self, remote: &str, bytes: u64) {
        let Some(index) = self.index(remote) else {
            return;
        };
        let (done, total, elapsed, eta, active, name) = {
            let mut shared = self
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            shared.rows[index].done = shared.rows[index].done.saturating_add(bytes);
            shared.rows[index].state = State::Uploading;
            shared.done = shared.done.saturating_add(bytes);
            let name = shared.rows[index].name.clone();
            let done = shared.rows[index].done;
            (
                done,
                shared.done,
                shared.started.elapsed(),
                shared.eta(),
                shared.active(),
                name,
            )
        };
        if let Some(bar) = self.rows.get(index) {
            bar.set_prefix(State::Uploading.label());
            bar.inc(bytes);
        }
        if let Some(bar) = &self.total_bar {
            bar.inc(bytes);
        }
        if self.mode == Mode::Json {
            let due = {
                let mut last = self
                    .last_json
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if last.elapsed() < JSON_INTERVAL {
                    false
                } else {
                    *last = Instant::now();
                    true
                }
            };
            if due {
                let seconds = elapsed.as_secs_f64().max(f64::EPSILON);
                let event = ProgressEvent {
                    schema_version: 1,
                    event: "progress",
                    file: name,
                    bytes: done,
                    total,
                    rate: total as f64 / seconds,
                    eta_seconds: eta.map_or(0, |eta| eta.as_secs()),
                    active,
                };
                emit(&event);
            }
        }
    }

    /// Mark one file as complete.
    pub fn file_done(&self, remote: &str, sha256: Option<&str>) {
        let Some(index) = self.index(remote) else {
            return;
        };
        let (name, size) = {
            let mut shared = self
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            shared.rows[index].state = State::Done;
            (shared.rows[index].name.clone(), shared.rows[index].size)
        };
        if let Some(bar) = self.rows.get(index) {
            bar.set_prefix(State::Done.label());
            bar.disable_steady_tick();
            bar.set_position(bar.length().unwrap_or(size));
        }
        if self.mode == Mode::Json {
            let event = FileDoneEvent {
                schema_version: 1,
                event: "file_done",
                file: name,
                bytes: size,
                sha256: sha256.map(ToString::to_string),
            };
            emit(&event);
        }
    }

    /// Mark one file as waiting again, after a failure.
    pub fn file_failed(&self, remote: &str) {
        let Some(index) = self.index(remote) else {
            return;
        };
        {
            let mut shared = self
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            shared.rows[index].state = State::Waiting;
        }
        if let Some(bar) = self.rows.get(index) {
            bar.set_prefix(State::Waiting.label());
            bar.disable_steady_tick();
        }
    }

    /// The per-connection rows for the segmented file, inserted below its row.
    pub fn connection_bars(&self, remote: &str, connections: usize) -> Vec<ProgressBar> {
        let Some(multi) = &self.multi else {
            return Vec::new();
        };
        let Some(index) = self.index(remote) else {
            return Vec::new();
        };
        let Some(row) = self.rows.get(index) else {
            return Vec::new();
        };
        (0..connections.max(1))
            .map(|worker| {
                let bar = ProgressBar::new(0);
                bar.set_style(
                    ProgressStyle::with_template(
                        "  {spinner} conn {prefix:>2} {bar:24} {bytes:>9}/{total_bytes:<9} \
                         {bytes_per_sec:>9} ETA {eta_precise} {msg}",
                    )
                    .expect("the connection progress template is valid")
                    .progress_chars("=>-"),
                );
                bar.set_prefix((worker + 1).to_string());
                bar.enable_steady_tick(Duration::from_millis(250));
                multi.insert_after(row, bar)
            })
            .collect()
    }

    /// Print the resume marker and count the bytes already on the server
    /// toward the total and the file row.
    pub fn resume(&self, remote: &str, from: u64, total: u64) {
        match self.mode {
            Mode::Off => {}
            Mode::Json => emit(&ResumeEvent {
                schema_version: 1,
                event: "resume",
                file: name_of(remote),
                bytes: from,
                total,
            }),
            Mode::Plain | Mode::Terminal => eprintln!("{}", resume_line(from, total)),
        }
        self.resume_from(remote, from);
    }

    fn resume_from(&self, remote: &str, from: u64) {
        let Some(index) = self.index(remote) else {
            return;
        };
        let done = {
            let mut shared = self
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(row) = shared.rows.get_mut(index) else {
                return;
            };
            row.done = from.min(row.size);
            let added = row.done;
            shared.done = shared.done.saturating_add(added);
            shared.done
        };
        if let Some(total_bar) = &self.total_bar {
            total_bar.set_position(done);
        }
        if let Some(bar) = self.rows.get(index) {
            bar.set_position(from);
        }
    }

    /// Report one adaptive change of the active connection count.
    pub fn connections(&self, from: u8, to: u8) {
        match self.mode {
            Mode::Off => {}
            Mode::Json => emit(&ConnectionsEvent {
                schema_version: 1,
                event: "connections",
                from,
                to,
            }),
            Mode::Plain | Mode::Terminal => {
                eprintln!("segment: connections {from} -> {to}");
            }
        }
    }

    /// Finish the view. Safe to call more than once.
    pub fn finish(&self) {
        if self.finished.swap(true, Ordering::SeqCst) {
            return;
        }
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self
            .ticker
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = handle.join();
        }
        let (done, total, elapsed) = {
            let shared = self
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            (shared.done, shared.total, shared.started.elapsed())
        };
        if self.mode == Mode::Plain {
            eprintln!("{}", plain_line(done, total, elapsed, Some(Duration::ZERO)));
        }
        if self.mode == Mode::Json {
            let seconds = elapsed.as_secs_f64();
            let event = DoneEvent {
                schema_version: 1,
                event: "done",
                bytes: done,
                seconds,
                rate: done as f64 / seconds.max(f64::EPSILON) / 1_000_000.0,
            };
            emit(&event);
        }
        if let Some(multi) = &self.multi {
            multi.clear().ok();
        }
    }
}

impl Drop for UploadProgress {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self
            .ticker
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = handle.join();
        }
    }
}

fn emit(event: &impl Serialize) {
    if let Ok(text) = serde_json::to_string(event) {
        eprintln!("{text}");
    }
}

fn name_of(remote: &str) -> String {
    remote.rsplit('/').next().unwrap_or(remote).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(name: &str, size: u64) -> UploadUnit {
        UploadUnit {
            local: std::path::PathBuf::from(format!("/local/{name}")),
            remote: format!("/srv/@1/addons/{name}"),
            size,
        }
    }

    #[test]
    fn the_mode_follows_the_flags_and_the_terminal() {
        assert_eq!(resolve(true, false, true), Mode::Terminal);
        assert_eq!(resolve(true, false, false), Mode::Plain);
        assert_eq!(resolve(false, false, true), Mode::Off);
        assert_eq!(resolve(false, true, false), Mode::Json);
        assert_eq!(resolve(true, true, true), Mode::Json);
    }

    #[test]
    fn the_resume_marker_names_the_percent_and_the_bytes() {
        assert_eq!(
            resume_line(32 * 1024 * 1024, 64 * 1024 * 1024),
            "segment: resuming at 50% (33554432 of 67108864 bytes)"
        );
        assert_eq!(resume_line(1, 3), "segment: resuming at 33% (1 of 3 bytes)");
    }

    #[test]
    fn the_plain_line_has_the_documented_shape() {
        let line = plain_line(
            14_100_000,
            33_600_000,
            Duration::from_millis(7_833),
            Some(Duration::from_secs(11)),
        );
        assert_eq!(
            line,
            "progress 42% 14.1/33.6 MB 1.8 MB/s elapsed 00:07 ETA 00:11"
        );
        let done = plain_line(
            33_600_000,
            33_600_000,
            Duration::from_secs(12),
            Some(Duration::ZERO),
        );
        assert!(done.contains("100%"), "{done}");
        assert!(done.contains("00:12"), "{done}");
    }

    #[test]
    fn the_fallback_line_names_the_kind_and_the_message() {
        let error = UksftaError::Input("the server said no".to_string());
        assert_eq!(
            fallback_line(&error),
            "segment: not used: Input: the server said no; using the whole-file path"
        );
        let error = UksftaError::Locked("another upload".to_string());
        assert!(fallback_line(&error).starts_with("segment: not used: Locked:"));
    }

    #[test]
    fn every_event_is_one_json_object_with_the_documented_keys() {
        let start = serde_json::to_value(StartEvent {
            schema_version: 1,
            event: "start",
            files: 2,
            bytes: 100,
        })
        .expect("start");
        assert_eq!(start["event"], "start");
        assert_eq!(start["files"], 2);
        assert_eq!(start["bytes"], 100);
        assert_eq!(start["schema_version"], 1);

        let progress = serde_json::to_value(ProgressEvent {
            schema_version: 1,
            event: "progress",
            file: "a.pbo".to_string(),
            bytes: 40,
            total: 60,
            rate: 1.5,
            eta_seconds: 3,
            active: 1,
        })
        .expect("progress");
        for key in [
            "file",
            "bytes",
            "total",
            "rate",
            "eta_seconds",
            "active",
            "schema_version",
            "event",
        ] {
            assert!(progress.get(key).is_some(), "missing {key}");
        }

        let done = serde_json::to_value(FileDoneEvent {
            schema_version: 1,
            event: "file_done",
            file: "a.pbo".to_string(),
            bytes: 100,
            sha256: Some("ab".to_string()),
        })
        .expect("file_done");
        assert_eq!(done["sha256"], "ab");

        let without = serde_json::to_value(FileDoneEvent {
            schema_version: 1,
            event: "file_done",
            file: "a.pbo".to_string(),
            bytes: 100,
            sha256: None,
        })
        .expect("file_done");
        assert!(without.get("sha256").is_none(), "sha256 is omitted");

        let finish = serde_json::to_value(DoneEvent {
            schema_version: 1,
            event: "done",
            bytes: 100,
            seconds: 2.0,
            rate: 0.05,
        })
        .expect("done");
        assert_eq!(finish["event"], "done");
        assert_eq!(finish["bytes"], 100);
        assert_eq!(finish["seconds"], 2.0);
        assert_eq!(finish["rate"], 0.05);
    }

    #[test]
    fn an_off_view_draws_nothing_and_finishes() {
        let view = UploadProgress::new(&[unit("a.pbo", 100), unit("b.pbo", 50)], Mode::Off);
        assert_eq!(view.connection_bars("/srv/@1/addons/a.pbo", 4).len(), 0);
        view.file_started("/srv/@1/addons/a.pbo");
        view.advance("/srv/@1/addons/a.pbo", 40);
        view.file_done("/srv/@1/addons/a.pbo", Some("ab"));
        view.file_failed("/srv/@1/addons/b.pbo");
        view.finish();
    }
}
