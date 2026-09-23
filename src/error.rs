//! Typed errors for the CLI. Each variant maps to one process exit code so
//! a caller and CI can tell an input failure from a failed check.

/// `Check` failures exit 1. Everything else is an input or environment
/// failure and exits 2, matching clap's usage-error code.
#[derive(Debug, thiserror::Error)]
pub enum UksftaError {
    /// Missing or unreadable required input.
    #[error("{0}")]
    Input(String),
    /// Input that exists but does not parse.
    #[error("{0}")]
    Parse(String),
    /// A check that ran and found a problem.
    #[error("{0}")]
    Check(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl UksftaError {
    pub fn exit_code(&self) -> i32 {
        match self {
            UksftaError::Check(_) => 1,
            UksftaError::Input(_) | UksftaError::Parse(_) | UksftaError::Io(_) => 2,
        }
    }
}
