use std::path::PathBuf;

use thiserror::Error;

/// Library-boundary errors with actionable user-facing messages where possible.
#[derive(Debug, Error)]
pub enum Error {
    #[error("path not found: {0}")]
    PathNotFound(PathBuf),

    #[error("not a file or directory: {0}")]
    InvalidPath(PathBuf),

    #[error("invalid repository input: {0}")]
    InvalidInput(String),

    #[error("git is not available on PATH")]
    GitNotFound,

    #[error("git clone failed: {0}")]
    GitClone(String),

    #[error("git failed: {0}")]
    Git(String),

    #[error("no typeable content found in repository")]
    NoChunks,

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("terminal error: {0}")]
    Terminal(String),

    #[error("purge cancelled")]
    PurgeCancelled,

    #[error("{0}")]
    Config(String),

    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Exit code for CLI.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::PurgeCancelled => 0,
            Self::PathNotFound(_)
            | Self::InvalidPath(_)
            | Self::InvalidInput(_)
            | Self::NoChunks => 2,
            Self::GitNotFound | Self::GitClone(_) | Self::Git(_) => 3,
            Self::Database(_) => 4,
            Self::Io(_) | Self::Terminal(_) | Self::Config(_) | Self::Message(_) => 1,
        }
    }
}
