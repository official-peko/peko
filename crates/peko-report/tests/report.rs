//! The report must carry everything a coding agent needs to act on a finding.

use peko_check::{engine, PekoConfig, Project};
use peko_report::{markdown, Report, ReportBuilder, Tier};
use peko_rules::{Platform, RuleDatabase, Severity};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn report(fixture: &str, platform: Platform) -> Report {
    let database = RuleDatabase::load_from_dir(root().join("rules")).unwrap();
    let config = PekoConfig::new(platform);
    let project = Project::load(&root().join("fixtures").join(fixture), &config).unwrap();
    let rules = database.active_rules(platform);
    let outcome = engine::run(&project, &rules, &config, None).unwrap();
    ReportBuilder::new(&database, Tier::Lint)
        .severity_threshold(config.severity_threshold)
        .build(&project, &outcome)
}

#[test]
fn a_failing_report_states_why_it_failed() {
    let report = report("ios-violating", Platform::Ios);
    assert!(!report.summary.pass);
    assert_eq!(report.exit_code(), 1);
    assert!(report.summary.by_severity.error > 0);
    assert_eq!(report.summary.total_findings, report.findings.len());
    assert_eq!(report.tier, Tier::Lint);
    assert_eq!(report.rule_database_version, "0.1.0");
}

#[test]
fn a_clean_report_passes() {
    let report = report("ios-compliant", Platform::Ios);
    assert!(report.summary.pass);
    assert_eq!(report.exit_code(), 0);
    assert_eq!(report.summary.total_findings, 0);
}

#[test]
fn every_finding_carries_the_policy_source_and_a_fix() {
    for finding in &report("android-violating", Platform::Android).findings {
        assert!(
            !finding.title.is_empty(),
            "{} has no title",
            finding.rule_id
        );
        assert!(
            !finding.message.is_empty(),
            "{} has no message",
            finding.rule_id
        );
        assert!(
            finding.source_policy.url.starts_with("https://"),
            "{} has no policy link",
            finding.rule_id
        );
        assert!(
            !finding.remediation.summary.is_empty(),
            "{} has no remediation",
            finding.rule_id
        );
        assert!(!finding.check_type.is_empty());
    }
}

#[test]
fn the_report_round_trips_through_json() {
    let report = report("ios-violating", Platform::Ios);
    let json = report.to_json().unwrap();
    let parsed: Report = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.summary.total_findings, report.summary.total_findings);
    assert_eq!(parsed.findings.len(), report.findings.len());

    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    for field in [
        "report_id",
        "timestamp",
        "tool_version",
        "rule_database_version",
        "tier",
        "platform",
        "project",
        "summary",
        "findings",
        "dependency_report",
        "coverage",
        "data_handling",
    ] {
        assert!(
            value.get(field).is_some(),
            "the report has no {field} field"
        );
    }
}

#[test]
fn the_report_states_what_it_did_not_check() {
    let report = report("ios-violating", Platform::Ios);
    assert!(!report.coverage.transitive_dependencies_checked);
    assert!(!report.coverage.interpretive_analysis_performed);
    assert!(report.coverage.rules_checked > 0);
    assert!(report.coverage.files_analyzed > 0);
}

#[test]
fn the_report_lists_direct_dependencies_it_could_not_identify() {
    let report = report("ios-violating", Platform::Ios);
    assert_eq!(report.dependency_report.checked_count, 2);
    // No knowledge base was passed, so no dependency check ran and nothing is
    // recorded as unknown.
    assert_eq!(report.dependency_report.flagged_count, 0);
}

#[test]
fn the_markdown_rendering_holds_the_verdict_and_the_findings() {
    let text = markdown::render(&report("ios-violating", Platform::Ios));
    assert!(text.starts_with("# Peko compliance report: FAIL"));
    assert!(text.contains("## Summary"));
    assert!(text.contains("## Findings"));
    assert!(text.contains("## Coverage"));
    assert!(text.contains("AAPL-API-001"));
    assert!(text.contains("**Fix:**"));
    assert!(text.contains("**Policy:**"));
    assert!(text.contains("It does not guarantee approval"));
}

#[test]
fn an_overridden_finding_does_not_fail_the_run() {
    let database = RuleDatabase::load_from_dir(root().join("rules")).unwrap();
    let mut config = PekoConfig::new(Platform::Ios);
    config.severity_threshold = Severity::Warning;
    config.overrides.push(peko_check::RuleOverride {
        rule_id: "AAPL-PRIV-010".parse().unwrap(),
        status: peko_check::OverrideStatus::Acknowledged,
        reason: "The privacy manifest ships in a separate framework target.".into(),
        acknowledged_by: None,
        acknowledged_at: None,
    });
    let project = Project::load(&root().join("fixtures/ios-violating"), &config).unwrap();
    let rules = database.active_rules(Platform::Ios);
    let outcome = engine::run(&project, &rules, &config, None).unwrap();
    let report = ReportBuilder::new(&database, Tier::Lint)
        .severity_threshold(Severity::Warning)
        .build(&project, &outcome);

    assert_eq!(report.summary.overridden, 1);
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.rule_id.to_string() == "AAPL-PRIV-010")
        .unwrap();
    assert!(finding.overridden);
    assert!(finding.override_reason.is_some());
}

/// Build an audit tier report, saying whether the model ran.
fn audit_report(fixture: &str, platform: Platform, interpretive_ran: bool) -> Report {
    let database = RuleDatabase::load_from_dir(root().join("rules")).unwrap();
    let config = PekoConfig::new(platform);
    let project = Project::load(&root().join("fixtures").join(fixture), &config).unwrap();
    let rules = database.active_rules(platform);
    let outcome = engine::run(&project, &rules, &config, None).unwrap();
    ReportBuilder::new(&database, Tier::Audit)
        .severity_threshold(config.severity_threshold)
        .interpretive_performed(interpretive_ran)
        .build_with_audit(&project, &outcome, &[])
}

#[test]
fn an_audit_whose_model_never_ran_is_not_a_pass() {
    // This is what a deployed run produced: every model call returned 400,
    // all 52 interpretive rules failed, the mechanical checks found nothing,
    // and the report came back clean. A developer reads that as "my app is
    // fine". The truth was "most of it was never read".
    let report = audit_report("ios-compliant", Platform::Ios, false);
    assert_eq!(report.summary.total_findings, 0, "nothing was found");
    assert!(
        !report.summary.pass,
        "an audit that did not run its interpretive half reported a pass"
    );
    assert_ne!(report.exit_code(), 0, "a gate reading this would let it through");
    assert!(
        report
            .coverage
            .warnings
            .iter()
            .any(|line| line.contains("interpretive rules did not run")),
        "the report gives no reason for the failure"
    );
}

#[test]
fn an_audit_that_did_run_and_found_nothing_still_passes() {
    // The guard must not turn every clean audit into a failure.
    let report = audit_report("ios-compliant", Platform::Ios, true);
    assert!(report.summary.pass);
    assert_eq!(report.exit_code(), 0);
    assert!(report.coverage.interpretive_analysis_performed);
}

#[test]
fn a_lint_is_unaffected_by_the_audit_guard() {
    // A lint never runs interpretive rules, and it must still pass clean.
    let report = report("ios-compliant", Platform::Ios);
    assert!(!report.coverage.interpretive_analysis_performed);
    assert!(report.summary.pass);
}
