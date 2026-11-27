//! Hash a remote file without writing it to disk.

use std::fmt::Write;
use std::path::Path;

use bytes::BytesMut;
use openssh_sftp_client::file::File;
use openssh_sftp_client::Sftp;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncSeekExt, SeekFrom};

use crate::error::UksftaError;

/// Hash the whole remote file and fail when its length differs.
///
/// # Errors
/// Returns `Input` for a read error or a length mismatch.
pub async fn hash_remote(
    sftp: &Sftp,
    path: &str,
    expected_len: u64,
    buffer: u32,
) -> Result<String, UksftaError> {
    hash(sftp, path, expected_len, buffer).await
}

/// Hash the first `len` bytes of the remote file. A resume compares it with
/// the local prefix before it continues.
///
/// # Errors
/// Returns `Input` for a read error or a file shorter than `len`.
pub async fn hash_remote_prefix(
    sftp: &Sftp,
    path: &str,
    len: u64,
    buffer: u32,
) -> Result<String, UksftaError> {
    hash(sftp, path, len, buffer).await
}

async fn hash(sftp: &Sftp, path: &str, len: u64, buffer: u32) -> Result<String, UksftaError> {
    let mut file = sftp
        .open(path)
        .await
        .map_err(|error| UksftaError::Input(format!("cannot read {path}: {error}")))?;
    let hasher = read_into_hasher(&mut file, path, len, buffer).await?;

    Ok(hex_digest(&hasher.finalize()))
}

async fn read_into_hasher(
    file: &mut File,
    path: &str,
    len: u64,
    buffer: u32,
) -> Result<Sha256, UksftaError> {
    let mut hasher = Sha256::new();
    let mut offset = 0u64;
    let want_max = buffer.max(4096);

    while offset < len {
        let want = u32::try_from((len - offset).min(u64::from(want_max))).unwrap_or(want_max);
        // The crate advances its own offset by the requested length, not by
        // the bytes returned, so the offset is set again for every read.
        file.seek(SeekFrom::Start(offset))
            .await
            .map_err(|error| UksftaError::Input(format!("cannot read {path}: {error}")))?;
        // Zero length with the wanted capacity: the read appends and reports
        // the real length, so a zeroed buffer of `want` length would double.
        let chunk = BytesMut::with_capacity(usize::try_from(want).unwrap_or(4096));
        let Some(data) = file
            .read(want, chunk)
            .await
            .map_err(|error| UksftaError::Input(format!("cannot read {path}: {error}")))?
        else {
            return Err(UksftaError::Input(format!(
                "the server holds {offset} bytes, expected {len}"
            )));
        };
        if data.is_empty() {
            return Err(UksftaError::Input(format!(
                "the server holds {offset} bytes, expected {len}"
            )));
        }
        offset += data.len() as u64;
        hasher.update(&data);
    }
    Ok(hasher)
}

/// Hash the first `len` bytes of the local file.
///
/// # Errors
/// Returns `Input` when the file cannot be read or is shorter than `len`.
pub fn hash_local_prefix(path: &Path, len: u64) -> Result<String, UksftaError> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .map_err(|error| UksftaError::Input(format!("cannot read {}: {error}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut left = len;
    while left > 0 {
        let want = usize::try_from(left.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = file.read(&mut buffer[..want]).map_err(|error| {
            UksftaError::Input(format!("cannot read {}: {error}", path.display()))
        })?;
        if read == 0 {
            return Err(UksftaError::Input(format!(
                "the local file ended early: {}",
                path.display()
            )));
        }
        hasher.update(&buffer[..read]);
        left -= read as u64;
    }
    Ok(hex_digest(&hasher.finalize()))
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}
