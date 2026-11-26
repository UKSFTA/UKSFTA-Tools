//! Hash a remote file without writing it to disk.

use std::fmt::Write;

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

/// Hash the first `len` bytes of the remote file. Wave 3 compares it with
/// the local prefix before a resume.
///
/// # Errors
/// Returns `Input` for a read error or a file shorter than `len`.
#[allow(dead_code)]
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

    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
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
