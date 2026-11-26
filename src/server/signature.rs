//! Read the authority name from a `.bisign` or `.bikey`.
//!
//! The file starts with a NUL-terminated authority name, then a 24-byte
//! key header, then the key data. A `.bisign` continues with signature
//! blocks, which this reader ignores. Only 4096 bytes are read.

use std::fs::File;
use std::io::Read;
use std::path::Path;

/// The most bytes read from a key file.
pub const MAX_READ: usize = 4096;
/// The longest authority name accepted.
const MAX_NAME: usize = 1024;
/// The key header after the name terminator.
const HEADER_LEN: usize = 24;
/// The bytes of the blob size that are not key data.
const BLOB_OVERHEAD: u32 = 20;
/// The RSA key algorithm marker.
const KEY_ALG: u32 = 0x0000_2400;
/// The key version byte.
const KEY_VERSION: u8 = 2;

/// The authority name and key metadata of a key file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyName {
    pub name: String,
    pub bits: u32,
    pub public: bool,
}

/// A key file that does not follow the documented layout.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyNameError {
    #[error("the file ends before the key header is complete")]
    Truncated,
    #[error("the authority name is longer than 1024 bytes")]
    NameTooLong,
    #[error("the authority name is not UTF-8")]
    NotUtf8,
    #[error("the authority name is empty or holds an unsafe character")]
    BadName,
    #[error("the key header is not a valid RSA key header")]
    BadHeader,
    #[error("the key blob size disagrees with the header")]
    BadSize,
}

/// Read the authority name from the bytes of a key file.
///
/// # Errors
/// Returns a `KeyNameError` when the name or the header does not follow
/// the documented layout.
pub fn read_key_name(bytes: &[u8]) -> Result<KeyName, KeyNameError> {
    let bytes = &bytes[..bytes.len().min(MAX_READ)];
    let name_end = find_name_end(bytes)?;
    let name = std::str::from_utf8(&bytes[..name_end]).map_err(|_| KeyNameError::NotUtf8)?;
    if !valid_name(name) {
        return Err(KeyNameError::BadName);
    }

    let header_start = name_end + 1;
    let header = bytes
        .get(header_start..header_start + HEADER_LEN)
        .ok_or(KeyNameError::Truncated)?;
    let parsed = parse_header(header)?;

    let data_len =
        usize::try_from(parsed.blob_size - BLOB_OVERHEAD).map_err(|_| KeyNameError::BadSize)?;
    let needed = header_start + HEADER_LEN + data_len;
    if bytes.len() < needed {
        return Err(KeyNameError::Truncated);
    }

    Ok(KeyName {
        name: name.to_string(),
        bits: parsed.bits,
        public: parsed.public,
    })
}

/// Read the authority name from a key file, up to [`MAX_READ`] bytes.
///
/// # Errors
/// Returns a `KeyNameError` when the file cannot be read or parsed.
pub fn read_key_file(path: &Path) -> Result<KeyName, KeyNameError> {
    let file = File::open(path).map_err(|_| KeyNameError::Truncated)?;
    let mut bytes = Vec::new();
    file.take(MAX_READ as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| KeyNameError::Truncated)?;
    read_key_name(&bytes)
}

fn find_name_end(bytes: &[u8]) -> Result<usize, KeyNameError> {
    let window = &bytes[..bytes.len().min(MAX_NAME + 1)];
    match window.iter().position(|byte| *byte == 0) {
        Some(end) if end <= MAX_NAME => Ok(end),
        Some(_) => Err(KeyNameError::NameTooLong),
        None if bytes.len() > MAX_NAME => Err(KeyNameError::NameTooLong),
        None => Err(KeyNameError::Truncated),
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("..")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-' | b' '))
}

struct Header {
    blob_size: u32,
    bits: u32,
    public: bool,
}

fn parse_header(header: &[u8]) -> Result<Header, KeyNameError> {
    let blob_size = read_u32_le(header, 0);
    let key_type = header[4];
    let version = header[5];
    let algorithm = read_u32_le(header, 8);
    let magic = &header[12..16];
    let bits = read_u32_le(header, 16);
    let exponent = read_u32_le(header, 20);

    let public = match key_type {
        0x06 => true,
        0x07 => false,
        _ => return Err(KeyNameError::BadHeader),
    };
    let expected_magic: &[u8; 4] = if public { b"RSA1" } else { b"RSA2" };
    if version != KEY_VERSION || algorithm != KEY_ALG || magic != expected_magic {
        return Err(KeyNameError::BadHeader);
    }
    if !(512..=16_384).contains(&bits) {
        return Err(KeyNameError::BadHeader);
    }
    if exponent <= 1 || exponent % 2 == 0 {
        return Err(KeyNameError::BadHeader);
    }
    if blob_size < BLOB_OVERHEAD {
        return Err(KeyNameError::BadSize);
    }
    let expected_blob = BLOB_OVERHEAD + bits / 8;
    if public && blob_size != expected_blob {
        return Err(KeyNameError::BadSize);
    }
    if !public && blob_size < expected_blob {
        return Err(KeyNameError::BadSize);
    }

    Ok(Header {
        blob_size,
        bits,
        public,
    })
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTHORITY: &str = "uksfta_1.0.1";

    fn key_file(authority: &str, public: bool) -> Vec<u8> {
        let bits: u32 = 1024;
        let blob_size: u32 = if public { 20 + bits / 8 } else { 596 };
        let data_len = (blob_size - 20) as usize;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(authority.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&blob_size.to_le_bytes());
        bytes.push(if public { 6 } else { 7 });
        bytes.push(2);
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&KEY_ALG.to_le_bytes());
        bytes.extend_from_slice(if public { b"RSA1" } else { b"RSA2" });
        bytes.extend_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(&65537u32.to_le_bytes());
        bytes.resize(bytes.len() + data_len, 0);
        bytes
    }

    #[test]
    fn a_public_key_yields_its_name_and_bits() {
        let key = read_key_name(&key_file(AUTHORITY, true)).unwrap();
        assert_eq!(key.name, AUTHORITY);
        assert_eq!(key.bits, 1024);
        assert!(key.public);
    }

    #[test]
    fn a_bisign_prefix_stops_after_the_key_blob() {
        let mut bytes = key_file(AUTHORITY, true);
        bytes.extend_from_slice(&[1, 2, 3, 4]);

        let key = read_key_name(&bytes).unwrap();
        assert_eq!(key.name, AUTHORITY);
    }

    #[test]
    fn a_private_key_is_accepted() {
        let key = read_key_name(&key_file(AUTHORITY, false)).unwrap();
        assert!(!key.public);
        assert_eq!(key.bits, 1024);
    }

    #[test]
    fn a_name_over_the_cap_is_refused() {
        let long = "a".repeat(5000);
        assert_eq!(
            read_key_name(&key_file(&long, true)).unwrap_err(),
            KeyNameError::NameTooLong
        );
    }

    #[test]
    fn a_traversal_name_is_refused() {
        assert_eq!(
            read_key_name(&key_file("../x", true)).unwrap_err(),
            KeyNameError::BadName
        );
    }

    #[test]
    fn a_bad_magic_is_refused() {
        let mut bytes = key_file(AUTHORITY, true);
        let magic = 12 + 1 + 12;
        bytes[magic..magic + 4].copy_from_slice(b"RSA9");
        assert_eq!(read_key_name(&bytes).unwrap_err(), KeyNameError::BadHeader);
    }

    #[test]
    fn a_truncated_header_is_refused() {
        let mut bytes = key_file(AUTHORITY, true);
        bytes.truncate(12 + 1 + 10);
        assert_eq!(read_key_name(&bytes).unwrap_err(), KeyNameError::Truncated);
    }

    #[test]
    fn a_blob_size_that_disagrees_with_bits_is_refused() {
        let mut bytes = key_file(AUTHORITY, true);
        let blob = 12 + 1;
        bytes[blob..blob + 4].copy_from_slice(&84u32.to_le_bytes());
        assert_eq!(read_key_name(&bytes).unwrap_err(), KeyNameError::BadSize);
    }

    #[test]
    fn a_bad_version_algorithm_or_bits_is_refused() {
        let header = 12 + 1;

        let mut version = key_file(AUTHORITY, true);
        version[header + 5] = 3;
        assert_eq!(
            read_key_name(&version).unwrap_err(),
            KeyNameError::BadHeader
        );

        let mut algorithm = key_file(AUTHORITY, true);
        algorithm[header + 8..header + 12].copy_from_slice(&0x2401u32.to_le_bytes());
        assert_eq!(
            read_key_name(&algorithm).unwrap_err(),
            KeyNameError::BadHeader
        );

        let mut bits = key_file(AUTHORITY, true);
        bits[header + 16..header + 20].copy_from_slice(&256u32.to_le_bytes());
        assert_eq!(read_key_name(&bits).unwrap_err(), KeyNameError::BadHeader);
    }

    #[test]
    fn an_even_exponent_is_refused() {
        let mut bytes = key_file(AUTHORITY, true);
        let exponent = 12 + 1 + 20;
        bytes[exponent..exponent + 4].copy_from_slice(&65536u32.to_le_bytes());
        assert_eq!(read_key_name(&bytes).unwrap_err(), KeyNameError::BadHeader);
    }

    #[test]
    fn a_file_that_ends_before_the_key_data_is_refused() {
        let mut bytes = key_file(AUTHORITY, true);
        bytes.truncate(bytes.len() - 1);
        assert_eq!(read_key_name(&bytes).unwrap_err(), KeyNameError::Truncated);
    }

    #[test]
    fn the_real_pack_key_is_read_when_it_is_present() {
        let path = Path::new("../UKSFTA-Maps/.hemttout/release/keys/uksfta_1.0.1.bikey");
        if !path.exists() {
            return;
        }
        let key = read_key_file(path).unwrap();
        assert_eq!(key.name, AUTHORITY);
        assert_eq!(key.bits, 1024);
        assert!(key.public);
    }
}
