//! The human readable rendering of a report.

use crate::schema::{Report, ReportFinding, Tier};
use peko_rules::Severity;
use std::fmt::Write;

/// Render a report as Markdown.
pub fn render(report: &Report) -> String {
    let mut out = String::new();
    write_header(&mut out, report);
    write_summary(&mut out, report);
    write_findings(&mut out, report);
    write_dependencies(&mut out, report);
    write_coverage(&mut out, report);
    out
}

fn severity_marker(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "ERROR",
        Severity::Warning => "WARNING",
        Severity::Info => "INFO",
    }
}

fn write_header(out: &mut String, report: &Report) {
    let verdict = if report.summary.pass { "PASS" } else { "FAIL" };
    let _ = writeln!(out, "# Peko compliance report: {verdict}");
    let _ = writeln!(out);
    let _ = writeln!(out, "| Field | Value |");
    let _ = writeln!(out, "|---|---|");
    let _ = writeln!(out, "| Project | {} |", report.project.name);
    if let Some(bundle_id) = &report.project.bundle_id {
        let _ = writeln!(out, "| Bundle id | `{bundle_id}` |");
    }
    if let Some(package_name) = &report.project.package_name {
        let _ = writeln!(out, "| Package name | `{package_name}` |");
    }
    let _ = writeln!(out, "| Platform | {} |", report.platform);
    let _ = writeln!(out, "| Tier | {} |", report.tier);
    let _ = writeln!(out, "| Rule database | {} |", report.rule_database_version);
    let _ = writeln!(out, "| Tool version | {} |", report.tool_version);
    let _ = writeln!(
        out,
        "| Report time | {} |",
        report.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
    );
    let _ = writeln!(out);
}

fn write_summary(out: &mut String, report: &Report) {
    let summary = &report.summary;
    let _ = writeln!(out, "## Summary");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{} findings: {} error, {} warning, {} info. {} findings are overridden.",
        summary.total_findings,
        summary.by_severity.error,
        summary.by_severity.warning,
        summary.by_severity.info,
        summary.overridden
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "The run fails at severity {} or above.",
        summary.severity_threshold
    );
    let _ = writeln!(out);
}

fn write_findings(out: &mut String, report: &Report) {
    if report.findings.is_empty() {
        let _ = writeln!(out, "## Findings");
        let _ = writeln!(out);
        let _ = writeln!(out, "No finding was raised.");
        let _ = writeln!(out);
        return;
    }

    let _ = writeln!(out, "## Findings");
    let _ = writeln!(out);
    for (category, findings) in report.by_category() {
        let _ = writeln!(out, "### {} ({})", category.title(), category.code());
        let _ = writeln!(out);
        for finding in findings {
            write_finding(out, finding);
        }
    }
}

fn write_finding(out: &mut String, finding: &ReportFinding) {
    let marker = severity_marker(finding.severity);
    let overridden = if finding.overridden {
        " (overridden)"
    } else {
        ""
    };
    let _ = writeln!(
        out,
        "#### {marker} {}: {}{overridden}",
        finding.rule_id, finding.title
    );
    let _ = writeln!(out);

    if let Some(confidence) = finding.confidence {
        let label = if confidence >= 0.8 {
            "high"
        } else if confidence >= 0.5 {
            "uncertain"
        } else {
            "low confidence"
        };
        let _ = writeln!(
            out,
            "> Interpretive finding. Confidence {confidence:.2} ({label}). A human must confirm it."
        );
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "{}", finding.message);
    let _ = writeln!(out);

    if let Some(location) = &finding.location {
        match location.line_start {
            Some(line) => {
                let _ = writeln!(out, "Location: `{}:{line}`", location.file);
            }
            None => {
                let _ = writeln!(out, "Location: `{}`", location.file);
            }
        }
        let _ = writeln!(out);
        if let Some(snippet) = &location.snippet {
            if !snippet.trim().is_empty() {
                let _ = writeln!(out, "```");
                let _ = writeln!(out, "{}", snippet.trim_end());
                let _ = writeln!(out, "```");
                let _ = writeln!(out);
            }
        }
    }

    let _ = writeln!(out, "{}", finding.description);
    let _ = writeln!(out);
    let _ = writeln!(out, "**Fix:** {}", finding.remediation.summary);
    let _ = writeln!(out);
    for example in &finding.remediation.examples {
        let _ = writeln!(out, "```");
        let _ = writeln!(out, "{example}");
        let _ = writeln!(out, "```");
        let _ = writeln!(out);
    }
    let _ = writeln!(
        out,
        "**Policy:** {} section {}, {}",
        finding.source_policy.document, finding.source_policy.section, finding.source_policy.url
    );
    let _ = writeln!(out);

    if let Some(metadata) = &finding.interpretive_metadata {
        if !metadata.alternative_interpretations.is_empty() {
            let _ = writeln!(out, "**Other readings of this rule:**");
            let _ = writeln!(out);
            for interpretation in &metadata.alternative_interpretations {
                let _ = writeln!(out, "- {interpretation}");
            }
            let _ = writeln!(out);
        }
    }

    if let Some(reason) = &finding.override_reason {
        let _ = writeln!(out, "**Override reason:** {reason}");
        let _ = writeln!(out);
    }
}

fn write_dependencies(out: &mut String, report: &Report) {
    let dependencies = &report.dependency_report;
    let _ = writeln!(out, "## Dependencies");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{} direct dependencies read. {} carry a flag. {} are not in the knowledge base.",
        dependencies.checked_count, dependencies.flagged_count, dependencies.unknown_count
    );
    let _ = writeln!(out);

    if !dependencies.flags.is_empty() {
        let _ = writeln!(out, "| Package | Flag | Severity | Description |");
        let _ = writeln!(out, "|---|---|---|---|");
        for flag in &dependencies.flags {
            let _ = writeln!(
                out,
                "| `{}` | {} | {} | {} |",
                flag.package, flag.flag_type, flag.severity, flag.description
            );
        }
        let _ = writeln!(out);
    }

    if !dependencies.unknown_dependencies.is_empty() {
        let _ = writeln!(out, "Dependencies with no knowledge base entry:");
        let _ = writeln!(out);
        for package in &dependencies.unknown_dependencies {
            let _ = writeln!(out, "- `{package}`");
        }
        let _ = writeln!(out);
    }
}

fn write_coverage(out: &mut String, report: &Report) {
    let coverage = &report.coverage;
    let _ = writeln!(out, "## Coverage");
    let _ = writeln!(out);
    let _ = writeln!(out, "- Rules checked: {}", coverage.rules_checked);
    let _ = writeln!(
        out,
        "- Rules skipped because the project holds no matching file: {}",
        coverage.rules_skipped
    );
    let _ = writeln!(out, "- Files analyzed: {}", coverage.files_analyzed);
    let _ = writeln!(
        out,
        "- Interpretive analysis: {}",
        if coverage.interpretive_analysis_performed {
            "performed"
        } else if report.tier == Tier::Lint {
            "not part of the lint tier"
        } else {
            "not performed"
        }
    );
    let _ = writeln!(
        out,
        "- Transitive dependencies: {}",
        if coverage.transitive_dependencies_checked {
            "checked"
        } else {
            "not checked in v1"
        }
    );
    if !coverage.assumed_facts.is_empty() {
        let _ = writeln!(
            out,
            "- Assumed, because nobody answered: {}",
            coverage.assumed_facts.join(", ")
        );
        let _ = writeln!(
            out,
            "  Answer any of these in `.pekorc.json` to replace the assumption."
        );
    }
    let _ = writeln!(out);

    if !coverage.warnings.is_empty() {
        let _ = writeln!(out, "### Warnings");
        let _ = writeln!(out);
        for warning in &coverage.warnings {
            let _ = writeln!(out, "- {warning}");
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "---");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "This report identifies known compliance risks. It does not guarantee approval, and it is not legal advice."
    );
}
