//! The report schema (specification section 5.4).
//!
//! JSON is the primary format. It is written for a coding agent to read:
//! every finding carries the rule reference, the location, the policy source,
//! and the remediation, so the agent needs no second lookup.

use chrono::{DateTime, Utc};
use peko_rules::{Category, Platform, RuleId, RuleType, Severity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Which tier produced the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Mechanical checks only. No language model.
    Lint,
    /// Mechanical checks plus interpretive analysis.
    Audit,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Lint => "lint",
            Tier::Audit => "audit",
        }
    }
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Tier {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "lint" => Ok(Tier::Lint),
            "audit" => Ok(Tier::Audit),
            other => Err(format!("unknown tier {other:?}")),
        }
    }
}

/// What the report says about the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub name: String,
    pub bundle_id: Option<String>,
    pub package_name: Option<String>,
}

/// Finding counts by severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SeverityCounts {
    pub error: usize,
    pub warning: usize,
    pub info: usize,
}

/// Finding counts by rule type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TypeCounts {
    pub mechanical: usize,
    pub interpretive: usize,
}

/// The headline of the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// True when no finding at or above the severity threshold survives an
    /// override.
    pub pass: bool,
    /// The threshold that decided `pass`.
    pub severity_threshold: Severity,
    pub total_findings: usize,
    pub by_severity: SeverityCounts,
    pub by_type: TypeCounts,
    pub overridden: usize,
    pub dependency_flags: usize,
}

/// Where a finding sits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub file: String,
    pub line_start: Option<usize>,
    pub line_end: Option<usize>,
    pub snippet: Option<String>,
}

/// The policy text behind a finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePolicy {
    pub document: String,
    pub section: String,
    pub url: String,
}

/// What to do about a finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemediationInfo {
    pub summary: String,
    pub examples: Vec<String>,
}

/// Extra fields that only an interpretive finding carries.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InterpretiveMetadata {
    pub interpretation_applied: Option<String>,
    pub alternative_interpretations: Vec<String>,
    pub forum_evidence_summary: Option<String>,
}

/// One finding as the report renders it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportFinding {
    pub finding_id: Uuid,
    pub rule_id: RuleId,
    pub rule_type: RuleType,
    pub category: Category,
    pub severity: Severity,
    /// Present only for an interpretive finding.
    pub confidence: Option<f32>,
    pub title: String,
    pub description: String,
    /// What the check observed in this project, in one sentence.
    pub message: String,
    /// The check that produced the finding.
    pub check_type: String,
    pub location: Option<Location>,
    pub source_policy: SourcePolicy,
    pub remediation: RemediationInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interpretive_metadata: Option<InterpretiveMetadata>,
    pub overridden: bool,
    pub override_reason: Option<String>,
}

/// One flagged dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyFlagReport {
    pub package: String,
    pub flag_type: String,
    pub description: String,
    pub severity: Severity,
    pub related_rule_ids: Vec<RuleId>,
}

/// What the dependency analyzer found.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DependencyReport {
    pub checked_count: usize,
    pub flagged_count: usize,
    pub unknown_count: usize,
    pub flags: Vec<DependencyFlagReport>,
    pub unknown_dependencies: Vec<String>,
}

/// What the run did and did not cover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    pub rules_checked: usize,
    /// Rules that did not apply to this platform or this project type.
    pub rules_skipped: usize,
    pub interpretive_analysis_performed: bool,
    pub transitive_dependencies_checked: bool,
    /// Files the checker read.
    pub files_analyzed: usize,
    /// Facts the run assumed because nobody answered them.
    ///
    /// A default is only set where one answer is overwhelmingly the common
    /// one, and it is never silent. A reader who disagrees with an assumption
    /// answers the fact in `.pekorc.json` and the run reads the answer instead.
    #[serde(default)]
    pub assumed_facts: Vec<String>,
    /// Non-fatal problems, for example a file that would not parse.
    pub warnings: Vec<String>,
}

/// The deletion promise that the API repeats in every response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataHandling {
    pub source_code_retained: bool,
    pub retention_policy: String,
}

impl Default for DataHandling {
    fn default() -> Self {
        Self {
            source_code_retained: false,
            retention_policy: "Source code is processed in memory and deleted when the report is generated. No source code is retained.".into(),
        }
    }
}

/// The complete report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub report_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub tool_version: String,
    pub rule_database_version: String,
    pub tier: Tier,
    pub platform: Platform,
    pub project: ProjectInfo,
    pub summary: Summary,
    pub findings: Vec<ReportFinding>,
    pub dependency_report: DependencyReport,
    pub coverage: Coverage,
    pub data_handling: DataHandling,
}

impl Report {
    /// Serialize the report as indented JSON.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    /// Findings grouped by category, each group ordered by severity.
    pub fn by_category(&self) -> BTreeMap<Category, Vec<&ReportFinding>> {
        let mut grouped: BTreeMap<Category, Vec<&ReportFinding>> = BTreeMap::new();
        for finding in &self.findings {
            grouped.entry(finding.category).or_default().push(finding);
        }
        for group in grouped.values_mut() {
            group.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.rule_id.cmp(&b.rule_id)));
        }
        grouped
    }

    /// The exit code that the CLI returns for this report.
    ///
    /// | Code | Meaning |
    /// |---|---|
    /// | 0 | Every check passed |
    /// | 1 | A finding sits at or above the severity threshold |
    pub fn exit_code(&self) -> i32 {
        i32::from(!self.summary.pass)
    }
}
