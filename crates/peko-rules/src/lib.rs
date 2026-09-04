//! The Peko rule schema and rule database.
//!
//! This crate defines the structured form of a compliance rule, the rule id
//! convention, the category taxonomy, and the loader for the versioned rule
//! database. It performs no analysis. The `peko-check` crate consumes it.

pub mod category;
pub mod db;
pub mod embedded;
pub mod error;
pub mod facts;
pub mod id;
pub mod platform;
pub mod schema;
pub mod validate;

pub use category::Category;
pub use db::{DatabaseManifest, RuleDatabase, RuleQuery};
pub use error::{Result, RuleError, ValidationIssue};
pub use id::RuleId;
pub use platform::Platform;
pub use schema::{
    ChangelogEntry, ConfigFile, DependencyFlagType, Detection, Ecosystem, Interpretation,
    Likelihood, ManifestFile, MechanicalCheck, Precondition, PrivacyManifestRequirement,
    Remediation, Rule, RuleStatus, RuleType, Severity, SourceRef, Target, ValueMatcher,
};
pub use validate::validate_rule;
