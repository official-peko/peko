//! Building a report from checker findings and the rule database.

use crate::schema::{
    Coverage, DataHandling, DependencyReport, InterpretiveMetadata, Location, ProjectInfo,
    RemediationInfo, Report, ReportFinding, SeverityCounts, SourcePolicy, Summary, Tier,
    TypeCounts,
};
use chrono::Utc;
use peko_check::{Finding, MechanicalOutcome, Project};
use peko_rules::{Rule, RuleDatabase, RuleType, Severity};
use uuid::Uuid;

/// The tool version reported to clients.
pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Assemble a report.
///
/// The builder joins each finding with its rule, so the report carries the
/// policy reference and the remediation that the finding alone does not hold.
pub struct ReportBuilder<'a> {
    database: &'a RuleDatabase,
    tier: Tier,
    threshold: Severity,
    interpretive_performed: bool,
}

impl<'a> ReportBuilder<'a> {
    pub fn new(database: &'a RuleDatabase, tier: Tier) -> Self {
        Self {
            database,
            tier,
            threshold: Severity::Warning,
            interpretive_performed: false,
        }
    }

    /// Set the severity that fails the run.
    #[must_use]
    pub fn severity_threshold(mut self, threshold: Severity) -> Self {
        self.threshold = threshold;
        self
    }

    /// Record whether the interpretive checker ran.
    ///
    /// An audit whose language model was unreachable sets this to false, and
    /// the report says that interpretive analysis did not run.
    #[must_use]
    pub fn interpretive_performed(mut self, performed: bool) -> Self {
        self.interpretive_performed = performed;
        self
    }

    /// Build the report.
    pub fn build(&self, project: &Project, outcome: &MechanicalOutcome) -> Report {
        self.build_from(project, &outcome.findings, outcome)
    }

    /// Build an audit tier report from both sets of findings.
    ///
    /// The two tiers produce the same kind of finding, so they merge into one
    /// list. A reader wants the whole picture in one place, and the
    /// `rule_type` field already says which tier raised each one.
    pub fn build_with_audit(
        &self,
        project: &Project,
        mechanical: &MechanicalOutcome,
        audit: &[peko_check::Finding],
    ) -> Report {
        let mut findings = mechanical.findings.clone();
        findings.extend_from_slice(audit);
        self.build_from(project, &findings, mechanical)
    }

    fn build_from(
        &self,
        project: &Project,
        raw: &[peko_check::Finding],
        outcome: &MechanicalOutcome,
    ) -> Report {
        let findings: Vec<ReportFinding> = raw
            .iter()
            .filter_map(|finding| self.render_finding(finding))
            .collect();

        let mut by_severity = SeverityCounts::default();
        let mut by_type = TypeCounts::default();
        let mut overridden = 0usize;
        let mut pass = true;

        for finding in &findings {
            match finding.severity {
                Severity::Error => by_severity.error += 1,
                Severity::Warning => by_severity.warning += 1,
                Severity::Info => by_severity.info += 1,
            }
            match finding.rule_type {
                RuleType::Mechanical => by_type.mechanical += 1,
                RuleType::Interpretive => by_type.interpretive += 1,
            }
            if finding.overridden {
                overridden += 1;
            } else if finding.severity >= self.threshold {
                pass = false;
            }
        }

        // An audit whose interpretive half never ran did not check what it
        // was asked to check. Reporting a pass for it is the worst answer
        // this tool can give: a developer reads "pass" as "my app is fine"
        // when the truth is "most of it was never read".
        //
        // This happened against the deployed service. Every model call
        // returned 400, all 52 rules failed, and the report came back clean
        // with no findings. A gate that reads `pass` must fail here.
        let incomplete = self.tier == Tier::Audit && !self.interpretive_performed;
        let pass = pass && !incomplete;

        let mut warnings: Vec<String> = project
            .warnings
            .iter()
            .chain(outcome.warnings.iter())
            .cloned()
            .collect();
        if incomplete {
            warnings.push(
                "The interpretive rules did not run, so this report covers \
                 the mechanical checks only. It is not a pass."
                    .to_string(),
            );
        }

        Report {
            report_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            tool_version: TOOL_VERSION.to_string(),
            rule_database_version: self.database.version().to_string(),
            tier: self.tier,
            platform: project.platform,
            project: ProjectInfo {
                name: project.name.clone(),
                bundle_id: project.bundle_id.clone(),
                package_name: project.package_name.clone(),
            },
            summary: Summary {
                pass,
                severity_threshold: self.threshold,
                total_findings: findings.len(),
                by_severity,
                by_type,
                overridden,
                dependency_flags: outcome.flagged_dependencies,
            },
            findings,
            dependency_report: DependencyReport {
                checked_count: outcome.checked_dependencies,
                flagged_count: outcome.flagged_dependencies,
                unknown_count: outcome.unknown_dependencies.len(),
                flags: Vec::new(),
                unknown_dependencies: outcome.unknown_dependencies.clone(),
            },
            coverage: Coverage {
                rules_checked: outcome.rules_checked,
                rules_skipped: outcome.rules_skipped,
                interpretive_analysis_performed: self.interpretive_performed,
                transitive_dependencies_checked: false,
                files_analyzed: project.file_count(),
                assumed_facts: outcome.assumed_facts.clone(),
                warnings,
            },
            data_handling: DataHandling::default(),
        }
    }

    /// Join one finding with its rule. A finding whose rule left the database
    /// is dropped, because the report cannot state its policy source.
    fn render_finding(&self, finding: &Finding) -> Option<ReportFinding> {
        let rule: &Rule = self.database.get(finding.rule_id)?;
        Some(ReportFinding {
            finding_id: finding.finding_id,
            rule_id: finding.rule_id,
            rule_type: finding.rule_type,
            category: rule.category,
            severity: finding.severity,
            confidence: finding.confidence,
            title: rule.title.clone(),
            description: rule.description.clone(),
            message: finding.message.clone(),
            check_type: finding.check_type.clone(),
            location: finding.location.as_ref().map(|location| Location {
                file: location.file.to_string_lossy().into_owned(),
                line_start: location.line_start,
                line_end: location.line_end,
                snippet: location.snippet.clone(),
            }),
            source_policy: SourcePolicy {
                document: rule.source.document.clone(),
                section: rule.source.section.clone(),
                url: rule.source.url.clone(),
            },
            remediation: RemediationInfo {
                summary: rule.remediation.summary.clone(),
                examples: rule.remediation.examples.clone(),
            },
            interpretive_metadata: (finding.rule_type == RuleType::Interpretive).then(|| {
                InterpretiveMetadata {
                    interpretation_applied: None,
                    alternative_interpretations: rule
                        .interpretations
                        .iter()
                        .map(|entry| entry.interpretation.clone())
                        .collect(),
                    forum_evidence_summary: (!rule.remediation.forum_insights.is_empty())
                        .then(|| rule.remediation.forum_insights.join(" ")),
                }
            }),
            overridden: finding.overridden,
            override_reason: finding.override_reason.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use peko_check::finding::Finding;

    fn database() -> RuleDatabase {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rules");
        RuleDatabase::load_from_dir(root).expect("the rule database must load")
    }

    fn project() -> Project {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/ios-compliant");
        let config = peko_check::PekoConfig::new(peko_rules::Platform::Ios);
        Project::load(&root, &config).expect("the fixture must load")
    }

    /// A finding carries a rule id, and the report carries the rule's words.
    /// A reader needs the title and the fix, and the finding holds neither.
    fn finding(database: &RuleDatabase, severity: Severity, overridden: bool) -> Finding {
        let rule = database
            .rules()
            .iter()
            .find(|rule| rule.is_mechanical())
            .expect("a mechanical rule");
        let mut value = Finding::mechanical(
            rule.rule_id,
            severity,
            "manifest_key_present",
            "a message".to_string(),
            None,
        );
        value.overridden = overridden;
        value
    }

    fn outcome(findings: Vec<Finding>) -> peko_check::engine::MechanicalOutcome {
        peko_check::engine::MechanicalOutcome {
            findings,
            ..Default::default()
        }
    }

    #[test]
    fn a_run_with_nothing_to_fix_passes() {
        let db = database();
        let report = ReportBuilder::new(&db, Tier::Lint).build(&project(), &outcome(Vec::new()));
        assert!(report.summary.pass);
        assert_eq!(report.summary.total_findings, 0);
    }

    #[test]
    fn a_finding_at_the_threshold_fails_the_run() {
        let db = database();
        let report = ReportBuilder::new(&db, Tier::Lint)
            .severity_threshold(Severity::Warning)
            .build(
                &project(),
                &outcome(vec![finding(&db, Severity::Warning, false)]),
            );
        assert!(!report.summary.pass);
    }

    #[test]
    fn a_finding_below_the_threshold_does_not() {
        let db = database();
        let report = ReportBuilder::new(&db, Tier::Lint)
            .severity_threshold(Severity::Error)
            .build(
                &project(),
                &outcome(vec![finding(&db, Severity::Warning, false)]),
            );
        assert!(report.summary.pass);
    }

    /// An acknowledged finding stays in the report and stops failing the run.
    /// Removing it would hide a decision somebody made, and failing on it
    /// would make acknowledging pointless.
    #[test]
    fn an_acknowledged_finding_is_listed_and_does_not_fail_the_run() {
        let db = database();
        let report = ReportBuilder::new(&db, Tier::Lint)
            .severity_threshold(Severity::Warning)
            .build(
                &project(),
                &outcome(vec![finding(&db, Severity::Error, true)]),
            );
        assert!(report.summary.pass);
        assert_eq!(report.summary.total_findings, 1);
        assert_eq!(report.summary.overridden, 1);
    }

    #[test]
    fn the_counts_add_up_to_the_findings() {
        let db = database();
        let findings = vec![
            finding(&db, Severity::Error, false),
            finding(&db, Severity::Warning, false),
            finding(&db, Severity::Info, false),
        ];
        let report = ReportBuilder::new(&db, Tier::Lint).build(&project(), &outcome(findings));
        let counts = report.summary.by_severity;
        assert_eq!(counts.error + counts.warning + counts.info, 3);
        assert_eq!(report.summary.total_findings, 3);
    }

    /// A finding naming a rule the database does not hold is dropped rather
    /// than rendered with empty words, because the report reads the title and
    /// the fix off the rule.
    #[test]
    fn a_finding_for_a_rule_that_is_gone_is_left_out() {
        let db = database();
        let mut orphan = finding(&db, Severity::Error, false);
        orphan.rule_id = "AAPL-PRIV-999".parse().expect("an id");
        let report = ReportBuilder::new(&db, Tier::Lint).build(&project(), &outcome(vec![orphan]));
        assert_eq!(report.summary.total_findings, 0);
    }

    /// Every report repeats the deletion promise, because a promise a caller
    /// cannot see is not one.
    #[test]
    fn every_report_carries_the_deletion_promise() {
        let db = database();
        let report = ReportBuilder::new(&db, Tier::Lint).build(&project(), &outcome(Vec::new()));
        assert!(!report.data_handling.source_code_retained);
        assert!(!report.data_handling.retention_policy.is_empty());
    }
}
