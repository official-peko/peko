//! Report generation for the Peko checking engine.
//!
//! JSON is the primary format, written for a coding agent to read. Markdown is
//! the secondary format, written for a person.

pub mod builder;
pub mod markdown;
pub mod schema;

pub use builder::{ReportBuilder, TOOL_VERSION};
pub use markdown::render;
pub use peko_rules::Severity;
pub use schema::{
    Coverage, DataHandling, DependencyReport, Location, ProjectInfo, Report, ReportFinding,
    Summary, Tier,
};
