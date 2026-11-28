//! The human summary of the files an upload sent.
//!
//! The segmented file reports exact seconds and rate. A whole file is sent
//! in a batch, so the batch time is attributed to it by byte share and both
//! values carry a `~`.

use std::path::Path;

use crate::util::human_size;

/// The width of the seconds column.
const SECONDS: usize = 10;
/// The width of the bytes column.
const BYTES: usize = 10;
/// The width of the rate column.
const RATE: usize = 8;
/// The width of the hash column.
const HASH: usize = 64;
/// The longest name column.
const NAME_CAP: usize = 28;

/// One file that reached the server.
#[derive(Debug, Clone, PartialEq)]
pub struct SentFile {
    pub name: String,
    pub remote: String,
    pub bytes: u64,
    pub seconds: f64,
    /// The rate in millions of bytes per second. Absent for a 0-byte file.
    pub rate_mbps: Option<f64>,
    /// True when the seconds and the rate are an estimate.
    pub estimated: bool,
    pub sha256: String,
}

impl SentFile {
    /// The row for a whole file. A generated file can be planned with a size
    /// of 0, so the local file's real size wins when the plan has none.
    pub fn whole(local: &Path, remote: &str, planned: u64, seconds: f64) -> Self {
        let sha256 = crate::util::file_sha256_with_buffer(local, crate::util::HASH_BUFFER_LARGE)
            .unwrap_or_else(|| "-".to_string());
        Self::whole_with_hash(local, remote, planned, seconds, sha256)
    }

    /// The row for a whole file whose exact digest and seconds the send
    /// produced.
    pub fn native(local: &Path, remote: &str, planned: u64, seconds: f64, sha256: String) -> Self {
        let mut row = Self::whole_with_hash(local, remote, planned, seconds, sha256);
        row.estimated = false;
        row
    }

    /// The row for a whole file whose local digest another thread already
    /// computed. `sha256` is `-` when the file could not be read.
    pub fn whole_with_hash(
        local: &Path,
        remote: &str,
        planned: u64,
        seconds: f64,
        sha256: String,
    ) -> Self {
        let bytes = if planned == 0 {
            std::fs::metadata(local).map_or(planned, |meta| meta.len())
        } else {
            planned
        };
        Self::new(
            remote.rsplit('/').next().unwrap_or(remote).to_string(),
            remote.to_string(),
            bytes,
            seconds,
            true,
            sha256,
        )
    }

    /// Build one row. A 0-byte file or a 0-second transfer has no rate.
    pub fn new(
        name: String,
        remote: String,
        bytes: u64,
        seconds: f64,
        estimated: bool,
        sha256: String,
    ) -> Self {
        let rate_mbps = (bytes > 0 && seconds > 0.0).then(|| bytes as f64 / seconds / 1_000_000.0);
        Self {
            name,
            remote,
            bytes,
            seconds,
            rate_mbps,
            estimated,
            sha256,
        }
    }
}

fn seconds_cell(file: &SentFile) -> String {
    if file.bytes == 0 {
        return "0.0 s".to_string();
    }
    if file.estimated {
        format!("{:.1} s~", file.seconds)
    } else {
        format!("{:.1} s", file.seconds)
    }
}

fn rate_cell(file: &SentFile) -> String {
    match file.rate_mbps {
        None => "-".to_string(),
        Some(rate) if file.estimated => format!("{rate:.2}~"),
        Some(rate) => format!("{rate:.2}"),
    }
}

/// The table lines for a finished upload.
pub fn render(files: &[SentFile], total_seconds: f64) -> Vec<String> {
    let mut lines = vec![format!("sent {} file(s):", files.len())];
    if files.is_empty() {
        return lines;
    }
    let width = files
        .iter()
        .map(|file| file.name.chars().count())
        .chain(["file".len(), "total".len()])
        .max()
        .unwrap_or(4)
        .min(NAME_CAP);
    lines.push(format!(
        "{:<width$}  {:>BYTES$}  {:>SECONDS$}  {:>RATE$}  {:<HASH$}  remote",
        "file", "bytes", "seconds", "MB/s", "sha256",
    ));
    for file in files {
        lines.push(format!(
            "{:<width$}  {:>BYTES$}  {:>SECONDS$}  {:>RATE$}  {:<HASH$}  {}",
            file.name,
            human_size(file.bytes),
            seconds_cell(file),
            rate_cell(file),
            file.sha256,
            file.remote,
        ));
    }
    let bytes: u64 = files.iter().map(|file| file.bytes).sum();
    let rate = if bytes == 0 || total_seconds <= 0.0 {
        "-".to_string()
    } else {
        format!("{:.2}", bytes as f64 / total_seconds / 1_000_000.0)
    };
    lines.push(format!(
        "{:<width$}  {:>BYTES$}  {:>SECONDS$}  {:>RATE$}",
        "total",
        human_size(bytes),
        format!("{total_seconds:.1} s"),
        rate,
    ));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segmented() -> SentFile {
        SentFile::new(
            "big.pbo".to_string(),
            "/srv/@1/addons/big.pbo".to_string(),
            64_000_000,
            8.0,
            false,
            "a".repeat(64),
        )
    }

    fn whole() -> SentFile {
        SentFile::new(
            "keys/u.bikey".to_string(),
            "/srv/@1/keys/u.bikey".to_string(),
            1_000,
            1.0,
            true,
            "b".repeat(64),
        )
    }

    #[test]
    fn the_table_shows_the_exact_segmented_row() {
        let lines = render(&[segmented()], 8.0);
        assert_eq!(lines[0], "sent 1 file(s):");
        assert_eq!(
            lines[2],
            format!(
                "big.pbo     64.0 MB       8.0 s      8.00  {}  /srv/@1/addons/big.pbo",
                "a".repeat(64)
            )
        );
    }

    #[test]
    fn a_whole_file_marks_the_estimate() {
        let lines = render(&[whole()], 1.0);
        assert!(lines[2].contains("1.0 s~"), "{}", lines[2]);
        assert!(lines[2].contains("0.00~"), "{}", lines[2]);
        assert!(!lines[3].contains('~'), "{}", lines[3]);
    }

    #[test]
    fn a_native_whole_file_is_not_an_estimate() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("big.pbo");
        std::fs::write(&path, [b'x'; 128]).unwrap();
        let row = SentFile::native(&path, "/srv/@1/addons/big.pbo", 128, 0.5, "d".repeat(64));
        assert!(!row.estimated);
        let lines = render(&[row], 0.5);
        assert!(lines[2].contains("0.5 s"), "{}", lines[2]);
        assert!(!lines[2].contains('~'), "{}", lines[2]);
    }

    #[test]
    fn a_generated_file_takes_its_real_size_and_hash() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("meta.cpp");
        std::fs::write(&path, [b'x'; 54]).unwrap();

        let file = SentFile::whole(&path, "/srv/@1/meta.cpp", 0, 0.0);
        assert_eq!(file.bytes, 54);
        assert_eq!(file.sha256, crate::util::file_sha256(&path).unwrap());
        assert_ne!(
            file.sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        let lines = render(&[file], 0.0);
        assert!(lines[2].contains("54 B"), "{}", lines[2]);
        assert!(lines[2].contains("  -  "), "{}", lines[2]);
        assert!(lines[2].contains(&crate::util::file_sha256(&path).unwrap()));
        assert!(
            lines.last().unwrap().ends_with(" -"),
            "total row: {:?}",
            lines.last()
        );
    }

    #[test]
    fn a_zero_byte_file_shows_no_rate() {
        let empty = SentFile::new(
            "meta.cpp".to_string(),
            "/srv/@1/meta.cpp".to_string(),
            0,
            0.0,
            true,
            "c".repeat(64),
        );
        let lines = render(&[empty], 0.1);
        assert!(lines[2].contains(" 0 B"), "{}", lines[2]);
        assert!(lines[2].contains("0.0 s"), "{}", lines[2]);
        assert!(lines[2].contains("  -  "), "{}", lines[2]);
        assert!(!lines[2].contains('~'), "{}", lines[2]);
    }

    #[test]
    fn the_total_row_uses_the_exact_totals() {
        let lines = render(&[segmented(), whole()], 9.5);
        let total = lines.last().unwrap();
        assert!(total.starts_with("total"), "{total}");
        assert!(total.contains("64.0 MB"), "{total}");
        assert!(total.contains("9.5 s"), "{total}");
        assert!(total.contains("6.74"), "{total}");
    }

    #[test]
    fn an_empty_send_has_no_table() {
        assert_eq!(render(&[], 0.0), vec!["sent 0 file(s):".to_string()]);
    }
}
