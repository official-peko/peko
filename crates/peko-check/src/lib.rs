//! The Peko mechanical checker.
//!
//! The crate loads a project from disk, runs the deterministic checks of the
//! validated mechanical rules against it, and returns findings. It calls no
//! language model. The `peko-report` crate turns findings into a report.

pub mod config;
pub mod declarations;
pub mod derive;
pub mod discovery;
pub mod engine;
pub mod error;
pub mod finding;
pub mod harvest;
pub mod knowledge;
pub mod matcher;
pub mod plan;
pub mod project;
pub mod source;

pub use config::{OverrideStatus, PekoConfig, RuleOverride, CONFIG_FILE};
pub use discovery::{discover, DiscoveredFile, Discovery};
pub use engine::{run, MechanicalOutcome};
pub use error::{CheckError, Result};
pub use finding::{Finding, Location};
pub use knowledge::{ComplianceFlag, KnowledgeBase, PackageEntry};
pub use project::{ConfigDocument, Project};
pub use source::SourceFile;
