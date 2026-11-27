//! The whole-upload progress view: one total, one row per planned file, and
//! the per-connection rows that the segmented file expands into.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicU8, Ordering};
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

    const fn code(self) -> u8 {
        match self {
            State::Waiting => 0,
            State::Uploading => 1,
            State::Done => 2,
        }
    }

    const fn from_code(code: u8) -> Self {
        match code {
            1 => State::Uploading,
            2 => State::Done,
            _ => State::Waiting,
        }
    }
}

/// One planned file. Every field a sender writes is atomic, so a range ack
/// costs no lock and every mode sees the same counters.
struct RowData {
    name: String,
    size: u64,
    done: AtomicU64,
    state: AtomicU8,
}

impl RowData {
    fn state(&self) -> State {
        State::from_code(self.state.load(Ordering::Relaxed))
    }

    fn set_state(&self, state: State) {
        self.state.store(state.code(), Ordering::Relaxed);
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
    data: Vec<RowData>,
    index: HashMap<String, usize>,
    total: u64,
    done: AtomicU64,
    started: Instant,
    multi: Option<MultiProgress>,
    total_bar: Option<ProgressBar>,
    bars: Vec<ProgressBar>,
    last_json: AtomicI64,
    stop: AtomicBool,
    finished: AtomicBool,
    ticker: Mutex<Option<JoinHandle<()>>>,
}

impl UploadProgress {
    /// Build the view. Nothing is drawn for [`Mode::Off`].
    pub fn new(units: &[UploadUnit], mode: Mode) -> Arc<Self> {
        let data: Vec<RowData> = units
            .iter()
            .map(|unit| RowData {
                name: name_of(&unit.remote),
                size: unit.size,
                done: AtomicU64::new(0),
                state: AtomicU8::new(State::Waiting.code()),
            })
            .collect();
        // The remote name maps to its row once, so a byte ack is one lookup
        // rather than a scan.
        let index: HashMap<String, usize> = units
            .iter()
            .enumerate()
            .map(|(position, unit)| (unit.remote.clone(), position))
            .collect();
        let total = data.iter().map(|row| row.size).sum();

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
            let bars = data
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
                    bar.set_prefix(row.state().label());
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
            data,
            index,
            total,
            done: AtomicU64::new(0),
            started: Instant::now(),
            multi,
            total_bar,
            bars,
            last_json: AtomicI64::new(-(JSON_INTERVAL.as_millis() as i64)),
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
        let event = StartEvent {
            schema_version: 1,
            event: "start",
            files: self.data.len(),
            bytes: self.total,
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
                    let (done, total) = view.totals();
                    eprintln!(
                        "{}",
                        plain_line(done, total, view.started.elapsed(), view.eta())
                    );
                }
            }
        });
        *self
            .ticker
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(handle);
    }

    /// The whole-upload bytes counted and expected.
    fn totals(&self) -> (u64, u64) {
        (self.done.load(Ordering::Relaxed), self.total)
    }

    /// The rows currently sending bytes.
    fn active(&self) -> usize {
        self.data
            .iter()
            .filter(|row| row.state() == State::Uploading)
            .count()
    }

    fn eta(&self) -> Option<Duration> {
        let done = self.done.load(Ordering::Relaxed);
        let elapsed = self.started.elapsed().as_secs_f64();
        if done == 0 || elapsed <= f64::EPSILON {
            return None;
        }
        let rate = done as f64 / elapsed;
        let left = self.total.saturating_sub(done) as f64 / rate;
        Some(Duration::from_secs_f64(left.max(0.0)))
    }

    fn index(&self, remote: &str) -> Option<usize> {
        self.index.get(remote).copied()
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
        self.data[index].set_state(State::Uploading);
        if let Some(bar) = self.bars.get(index) {
            bar.set_prefix(State::Uploading.label());
            bar.enable_steady_tick(Duration::from_millis(250));
        }
    }

    /// Count acknowledged bytes for one file.
    pub fn advance(&self, remote: &str, bytes: u64) {
        let Some(index) = self.index(remote) else {
            return;
        };
        let row = &self.data[index];
        let done = add(&row.done, bytes);
        row.set_state(State::Uploading);
        let total = add(&self.done, bytes);
        if let Some(bar) = self.bars.get(index) {
            bar.set_prefix(State::Uploading.label());
            bar.inc(bytes);
        }
        if let Some(bar) = &self.total_bar {
            bar.inc(bytes);
        }
        if self.mode != Mode::Json || !self.json_due() {
            return;
        }
        let seconds = self.started.elapsed().as_secs_f64().max(f64::EPSILON);
        let event = ProgressEvent {
            schema_version: 1,
            event: "progress",
            file: row.name.clone(),
            bytes: done,
            total,
            rate: total as f64 / seconds,
            eta_seconds: self.eta().map_or(0, |eta| eta.as_secs()),
            active: self.active(),
        };
        emit(&event);
    }

    /// True once per [`JSON_INTERVAL`], for one caller only.
    fn json_due(&self) -> bool {
        let now = self.started.elapsed().as_millis() as i64;
        let interval = JSON_INTERVAL.as_millis() as i64;
        let previous = self.last_json.load(Ordering::Relaxed);
        if now - previous < interval {
            return false;
        }
        self.last_json
            .compare_exchange(previous, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    }

    /// Mark one file as complete.
    pub fn file_done(&self, remote: &str, sha256: Option<&str>) {
        let Some(index) = self.index(remote) else {
            return;
        };
        let row = &self.data[index];
        row.set_state(State::Done);
        if let Some(bar) = self.bars.get(index) {
            bar.set_prefix(State::Done.label());
            bar.disable_steady_tick();
            bar.set_position(bar.length().unwrap_or(row.size));
        }
        if self.mode == Mode::Json {
            let event = FileDoneEvent {
                schema_version: 1,
                event: "file_done",
                file: row.name.clone(),
                bytes: row.size,
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
        self.data[index].set_state(State::Waiting);
        if let Some(bar) = self.bars.get(index) {
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
        let Some(row) = self.bars.get(index) else {
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
        let row = &self.data[index];
        let done = from.min(row.size);
        row.done.store(done, Ordering::Relaxed);
        let total = add(&self.done, done);
        if let Some(total_bar) = &self.total_bar {
            total_bar.set_position(total);
        }
        if let Some(bar) = self.bars.get(index) {
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
        let (done, total) = self.totals();
        let elapsed = self.started.elapsed();
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

/// Add bytes to a counter and stop at the maximum.
fn add(counter: &AtomicU64, bytes: u64) -> u64 {
    match counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(bytes))
    }) {
        Ok(previous) | Err(previous) => previous.saturating_add(bytes),
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

    /// Every mode counts the same bytes and the same states; only the output
    /// differs.
    #[test]
    fn every_mode_keeps_the_same_view() {
        let mut views = Vec::new();
        for mode in [Mode::Off, Mode::Plain, Mode::Json] {
            let view = UploadProgress::new(&[unit("a.pbo", 100), unit("b.pbo", 50)], mode);
            view.file_started("/srv/@1/addons/a.pbo");
            view.advance("/srv/@1/addons/a.pbo", 40);
            view.advance("/srv/@1/addons/a.pbo", 60);
            view.file_done("/srv/@1/addons/a.pbo", Some("ab"));
            view.advance("/srv/@1/addons/b.pbo", 25);
            views.push((view.totals(), row_state(&view, 0), row_state(&view, 1)));
        }
        assert_eq!(
            views[0],
            ((125, 150), (100, State::Done), (25, State::Uploading))
        );
        assert_eq!(views[0], views[1]);
        assert_eq!(views[1], views[2]);
    }

    fn row_state(view: &UploadProgress, index: usize) -> (u64, State) {
        let row = &view.data[index];
        (row.done.load(Ordering::Relaxed), row.state())
    }

    #[test]
    fn the_active_count_follows_the_row_states() {
        let view = UploadProgress::new(
            &[unit("a.pbo", 10), unit("b.pbo", 10), unit("c.pbo", 10)],
            Mode::Off,
        );
        assert_eq!(view.active(), 0);
        view.file_started("/srv/@1/addons/a.pbo");
        view.file_started("/srv/@1/addons/b.pbo");
        assert_eq!(view.active(), 2);
        view.advance("/srv/@1/addons/a.pbo", 5);
        assert_eq!(view.active(), 2, "an ack keeps the row active");
        view.file_done("/srv/@1/addons/a.pbo", None);
        assert_eq!(view.active(), 1);
        view.file_failed("/srv/@1/addons/b.pbo");
        assert_eq!(view.active(), 0);
    }

    #[test]
    fn an_unknown_remote_changes_nothing() {
        let view = UploadProgress::new(&[unit("a.pbo", 10)], Mode::Off);
        view.advance("/srv/@1/addons/other.pbo", 5);
        view.file_started("/srv/@1/addons/other.pbo");
        view.file_done("/srv/@1/addons/other.pbo", None);
        assert_eq!(view.totals(), (0, 10));
        assert_eq!(row_state(&view, 0), (0, State::Waiting));
    }

    #[test]
    fn one_json_event_per_interval() {
        let view = UploadProgress::new(&[unit("a.pbo", 10)], Mode::Off);
        assert!(view.json_due(), "the first ack is always due");
        assert!(!view.json_due(), "the next ack is inside the interval");
    }
}
