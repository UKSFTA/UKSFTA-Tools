//! The native SSH/SFTP transport.
//!
//! Wave 1 proves connect, key auth, agent auth and strict host key checking.
//! The segmented writer and the upload hook arrive in Wave 2, so the
//! transport is not yet reachable from the CLI. Keep the allow until then.

#![allow(dead_code)]

pub mod connect;
pub mod known_hosts;

#[cfg(test)]
mod tests;
