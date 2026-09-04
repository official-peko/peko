//! Parse error types.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{path} is not a valid property list: {source}")]
    Plist {
        path: PathBuf,
        #[source]
        source: plist::Error,
    },

    #[error("{path} is not valid XML: {source}")]
    Xml {
        path: PathBuf,
        #[source]
        source: roxmltree::Error,
    },

    #[error("{path} is not valid JSON: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("invalid key path {path:?}: {reason}")]
    InvalidKeyPath { path: String, reason: String },

    #[error("{path} is not a valid Xcode project: {source}")]
    OpenStep {
        path: PathBuf,
        #[source]
        source: crate::openstep::OpenStepError,
    },

    #[error("{path} has no root element")]
    EmptyDocument { path: PathBuf },

    #[error("{path} is not a readable app bundle: {reason}")]
    Bundle { path: PathBuf, reason: String },
}

pub type Result<T> = std::result::Result<T, ParseError>;
