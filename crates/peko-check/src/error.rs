//! Checker error types.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("project root {path} is not a directory")]
    NotADirectory { path: PathBuf },

    #[error("invalid .pekorc.json at {path}: {source}")]
    Config {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("invalid glob {pattern:?}: {source}")]
    Glob {
        pattern: String,
        #[source]
        source: globset::Error,
    },

    #[error("rule {rule_id} holds an unusable check: {reason}")]
    BadCheck { rule_id: String, reason: String },

    #[error(transparent)]
    Parse(#[from] peko_parse::ParseError),
}

pub type Result<T> = std::result::Result<T, CheckError>;
