//! Findings produced by the checker.
//!
//! A finding names the rule, where the problem is, and what the check
//! observed. The report crate joins it with the rule to render the policy
//! reference and the remediation.

use peko_rules::{RuleId, RuleType, Severity};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Where a finding sits in the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    /// The path relative to the project root.
    pub file: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_end: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

impl Location {
    /// A location that names a file only.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            file: path.into(),
            line_start: None,
            line_end: None,
            snippet: None,
        }
    }

    /// A location that names a file, a line, and the surrounding source.
    pub fn line(path: impl Into<PathBuf>, line: usize, snippet: impl Into<String>) -> Self {
        Self {
            file: path.into(),
            line_start: Some(line),
            line_end: Some(line),
            snippet: Some(snippet.into()),
        }
    }
}

/// One compliance concern found by a check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub finding_id: Uuid,
    pub rule_id: RuleId,
    pub rule_type: RuleType,
    /// The severity after any confidence downgrade.
    pub severity: Severity,
    /// The confidence of an interpretive finding. Always `None` for a
    /// mechanical finding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// What the check observed, in one sentence.
    pub message: String,
    /// The check that produced the finding, for example `manifest_key_absent`.
    pub check_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    /// True when `.pekorc.json` acknowledges this rule.
    pub overridden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_reason: Option<String>,
}

impl Finding {
    /// Build a mechanical finding.
    pub fn mechanical(
        rule_id: RuleId,
        severity: Severity,
        check_type: impl Into<String>,
        message: impl Into<String>,
        location: Option<Location>,
    ) -> Self {
        Self {
            finding_id: Uuid::new_v4(),
            rule_id,
            rule_type: RuleType::Mechanical,
            severity,
            confidence: None,
            message: message.into(),
            check_type: check_type.into(),
            location,
            overridden: false,
            override_reason: None,
        }
    }

    /// True when this finding counts toward a failing run.
    pub fn counts_toward_failure(&self, threshold: Severity) -> bool {
        !self.overridden && self.severity >= threshold
    }
}
