//! Error types for rule loading and validation.

use std::path::PathBuf;

/// An error raised while a rule database is loaded or validated.
#[derive(Debug, thiserror::Error)]
pub enum RuleError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("invalid rule id {id:?}: {reason}")]
    InvalidRuleId { id: String, reason: String },

    #[error("invalid database version {version:?}: {source}")]
    InvalidVersion {
        version: String,
        #[source]
        source: semver::Error,
    },

    #[error("duplicate rule id {id}: defined in {first} and {second}")]
    DuplicateRuleId {
        id: String,
        first: PathBuf,
        second: PathBuf,
    },

    #[error("rule database manifest not found at {path}")]
    ManifestNotFound { path: PathBuf },

    #[error("rule {id} failed validation: {reason}")]
    Invalid { id: String, reason: String },

    #[error("invalid regex in rule {id}: {source}")]
    InvalidRegex {
        id: String,
        #[source]
        source: regex::Error,
    },
}

/// A validation problem found in a rule. Validation collects every problem
/// before it returns, so an author can correct them in one pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    /// The rule that holds the problem.
    pub rule_id: String,
    /// The field path inside the rule, for example `detection.mechanical_checks[0]`.
    pub field: String,
    /// A description of the problem.
    pub message: String,
}

impl std::fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} [{}]: {}", self.rule_id, self.field, self.message)
    }
}

pub type Result<T> = std::result::Result<T, RuleError>;
