//! The prompted password, held so it cannot be printed.

use std::io::BufRead;

use crate::error::UksftaError;

/// A password read from the controlling terminal.
///
/// It implements no `Debug`, `Display`, `Clone` or `Serialize`, so a log
/// line, a format string or a JSON report cannot carry it. The test module
/// proves that with a static assertion.
pub struct Secret(String);

impl Secret {
    /// The bytes, for the one authentication call that needs them.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // The bytes are NOT zeroised. `String` gives no way to overwrite the
        // buffer without `unsafe`, and this build adds no `zeroize`
        // dependency. The threat model excludes an attacker who can read this
        // process's memory, so the allocation may keep the password until the
        // kernel reuses it.
    }
}

/// Read one line and return it without its line ending.
///
/// # Errors
/// Returns `Input` when the terminal cannot be read or the entry is empty.
pub fn read_password(reader: &mut impl BufRead) -> Result<Secret, UksftaError> {
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|error| {
        UksftaError::Input(format!(
            "cannot read the terminal; use --identity or ssh-agent: {error}"
        ))
    })?;
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    if line.is_empty() {
        return Err(UksftaError::Input("no password entered".to_string()));
    }
    Ok(Secret(line))
}

/// Prompt on the controlling terminal with echo disabled.
///
/// # Errors
/// Returns `Input` when the terminal cannot be read or the entry is empty.
pub fn prompt(host: &str, user: &str) -> Result<Secret, UksftaError> {
    let text =
        rpassword::prompt_password(format!("Password for {user}@{host}: ")).map_err(|error| {
            UksftaError::Input(format!(
                "cannot read the terminal; use --identity or ssh-agent: {error}"
            ))
        })?;
    let mut reader = std::io::Cursor::new(text.into_bytes());
    read_password(&mut reader)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(input: &str) -> Result<Secret, UksftaError> {
        let mut reader = std::io::Cursor::new(input.as_bytes().to_vec());
        read_password(&mut reader)
    }

    #[test]
    fn the_line_ending_is_removed_and_spaces_stay() {
        assert_eq!(read("hunter two\n").unwrap().expose(), "hunter two");
        assert_eq!(read("hunter two\r\n").unwrap().expose(), "hunter two");
        assert_eq!(read("  spaced  ").unwrap().expose(), "  spaced  ");
    }

    #[test]
    fn an_empty_entry_is_refused() {
        for input in ["", "\n", "\r\n"] {
            match read(input) {
                Ok(_) => panic!("an empty entry must be refused"),
                Err(error) => assert_eq!(error.to_string(), "no password entered"),
            }
        }
    }

    // The proof: the type cannot leave through a formatter, a clone or a
    // serialised report.
    static_assertions::assert_not_impl_any!(
        Secret: std::fmt::Display, std::fmt::Debug, Clone, serde::Serialize
    );

    #[test]
    fn a_short_or_missing_reader_is_reported() {
        struct Failing;
        impl std::io::Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("no tty"))
            }
        }
        let mut reader = std::io::BufReader::new(Failing);
        assert!(read_password(&mut reader).is_err());
    }
}
