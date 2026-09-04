//! The report as SARIF, so findings land on the pull request.
//!
//! A report a reviewer has to go and read is a report nobody reads. GitHub
//! Code Scanning takes SARIF and puts each finding on the line it belongs to,
//! in the diff, where somebody is already looking.
//!
//! Three things matter to whoever reads it there, and each one is easy to get
//! wrong:
//!
//! A stable rule id. Code Scanning tracks a finding across pushes by that id,
//! so a fix closes the alert instead of opening a second one. The rule id is
//! used, never the finding id, which changes every run.
//!
//! A path relative to the repository root. An absolute path from a build
//! machine matches no file on GitHub, and the finding appears attached to
//! nothing.
//!
//! An honest severity. Code Scanning fails a check on `error`. A warning that
//! reports as an error stops a merge that should proceed, and a tool that does
//! that twice gets removed from the pipeline.

use crate::schema::{Report, ReportFinding};
use peko_rules::Severity;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// The SARIF version this writes.
pub const SCHEMA_VERSION: &str = "2.1.0";

/// The schema that describes the format.
pub const SCHEMA_URL: &str =
    "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json";

/// How a severity reads to Code Scanning.
///
/// `error` fails the check. Everything else informs. A warning that reports as
/// an error stops a merge that should proceed.
fn level_for(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "note",
    }
}

/// The path Code Scanning can match against the repository.
///
/// A leading `./` or a `/` prefix makes GitHub attach the finding to nothing,
/// because the path no longer matches a file in the tree.
fn relative_path(file: &str) -> String {
    file.trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

/// One SARIF rule, described once and referenced by every finding that uses it.
fn rule_object(finding: &ReportFinding) -> Value {
    let mut help = finding.remediation.summary.clone();
    if !finding.remediation.examples.is_empty() {
        help.push_str("\n\n");
        help.push_str(&finding.remediation.examples.join("\n\n"));
    }
    json!({
        "id": finding.rule_id.to_string(),
        "name": finding.rule_id.to_string(),
        "shortDescription": { "text": finding.title },
        "fullDescription": { "text": finding.description },
        "help": { "text": help },
        "helpUri": finding.source_policy.url,
        "properties": {
            "category": finding.category.to_string(),
            "policyDocument": finding.source_policy.document,
            "policySection": finding.source_policy.section,
            // Somebody reading an alert wants to know whether a file proved
            // this or a model judged it. The two deserve different trust.
            "ruleType": format!("{:?}", finding.rule_type).to_lowercase(),
        },
        "defaultConfiguration": { "level": level_for(finding.severity) },
    })
}

/// One SARIF result.
fn result_object(finding: &ReportFinding) -> Value {
    let mut result = json!({
        "ruleId": finding.rule_id.to_string(),
        "level": level_for(finding.severity),
        "message": { "text": finding.message },
    });

    if let Some(location) = &finding.location {
        // A region needs a start line. SARIF counts from one, and a zero makes
        // GitHub drop the location without saying so.
        let mut region = serde_json::Map::new();
        if let Some(start) = location.line_start.filter(|line| *line > 0) {
            region.insert("startLine".to_string(), json!(start));
            if let Some(end) = location.line_end.filter(|line| *line >= start) {
                region.insert("endLine".to_string(), json!(end));
            }
        }
        let mut physical = json!({
            "artifactLocation": {
                "uri": relative_path(&location.file),
                "uriBaseId": "%SRCROOT%",
            }
        });
        if !region.is_empty() {
            physical["region"] = Value::Object(region);
        }
        result["locations"] = json!([{ "physicalLocation": physical }]);
    }

    if let Some(confidence) = finding.confidence {
        // An interpretive finding is a judgement. Saying how sure it is lets a
        // reader weigh it rather than take it as fact.
        result["properties"] = json!({ "confidence": confidence });
    }
    result
}

/// Render a report as SARIF.
///
/// An overridden finding is left out. Somebody acknowledged it on purpose, and
/// an alert that comes back after that teaches people to ignore alerts.
#[must_use]
pub fn render(report: &Report) -> Value {
    let shown: Vec<&ReportFinding> = report
        .findings
        .iter()
        .filter(|finding| !finding.overridden)
        .collect();

    // One rule object per rule id, in a stable order. A map keyed by id also
    // stops a rule appearing twice when several findings share it.
    let mut rules: BTreeMap<String, Value> = BTreeMap::new();
    for finding in &shown {
        rules
            .entry(finding.rule_id.to_string())
            .or_insert_with(|| rule_object(finding));
    }

    json!({
        "$schema": SCHEMA_URL,
        "version": SCHEMA_VERSION,
        "runs": [{
            "tool": {
                "driver": {
                    "name": "Peko",
                    "informationUri": "https://github.com/peko-ui/peko",
                    "version": report.tool_version,
                    "semanticVersion": report.tool_version,
                    "rules": rules.into_values().collect::<Vec<_>>(),
                }
            },
            "results": shown.iter().map(|finding| result_object(finding)).collect::<Vec<_>>(),
            "properties": {
                "ruleDatabaseVersion": report.rule_database_version,
                "tier": format!("{:?}", report.tier).to_lowercase(),
            },
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real report, produced by running the engine on a fixture project.
    ///
    /// A hand written report drifts from the real shape the moment a field is
    /// added, and then these tests pass against a document the tool never
    /// produces.
    fn sample() -> Report {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("the repository root");
        let database = peko_rules::embedded::database().expect("the database loads");
        let config = peko_check::config::PekoConfig::load_or_default(
            &root.join("fixtures/ios-violating"),
            peko_rules::Platform::Ios,
        )
        .expect("the config loads");
        let project =
            peko_check::project::Project::load(&root.join("fixtures/ios-violating"), &config)
                .expect("the fixture loads");
        let rules = database.active_rules(config.platform);
        let outcome =
            peko_check::engine::run(&project, &rules, &config, None).expect("the run finishes");
        crate::ReportBuilder::new(&database, crate::Tier::Lint)
            .severity_threshold(config.severity_threshold)
            .build(&project, &outcome)
    }

    fn results(sarif: &Value) -> &Vec<Value> {
        sarif["runs"][0]["results"].as_array().expect("results")
    }

    #[test]
    fn every_severity_maps_to_the_level_code_scanning_expects() {
        // Code Scanning fails a check on error. A warning that reports as one
        // stops a merge that should proceed, and a tool that does that twice
        // gets removed from the pipeline.
        assert_eq!(level_for(Severity::Error), "error");
        assert_eq!(level_for(Severity::Warning), "warning");
        assert_eq!(level_for(Severity::Info), "note");

        // And the report's own findings carry those levels through.
        let report = sample();
        let sarif = render(&report);
        for (finding, result) in report
            .findings
            .iter()
            .filter(|f| !f.overridden)
            .zip(results(&sarif))
        {
            assert_eq!(result["level"], level_for(finding.severity));
        }
    }

    #[test]
    fn an_overridden_finding_does_not_come_back() {
        // Somebody acknowledged it on purpose. An alert that reappears after
        // that teaches people to ignore alerts.
        let mut report = sample();
        assert!(!report.findings.is_empty(), "the fixture raised nothing");
        let silenced = report.findings[0].rule_id.to_string();
        report.findings[0].overridden = true;
        let sarif = render(&report);
        assert!(
            !results(&sarif).iter().any(|r| r["ruleId"] == silenced),
            "the overridden finding was included"
        );
    }

    #[test]
    fn the_path_is_relative_so_github_can_match_it() {
        // An absolute path from a build machine matches no file on GitHub, and
        // the finding then appears attached to nothing.
        let sarif = render(&sample());
        for result in results(&sarif) {
            let Some(uri) =
                result["locations"][0]["physicalLocation"]["artifactLocation"]["uri"].as_str()
            else {
                continue;
            };
            assert!(
                !uri.starts_with('/'),
                "an absolute path reached SARIF: {uri}"
            );
            assert!(!uri.starts_with("./"), "a ./ prefix reached SARIF: {uri}");
        }
    }

    #[test]
    fn the_prefix_is_stripped_from_a_path() {
        assert_eq!(relative_path("./App/View.swift"), "App/View.swift");
        assert_eq!(relative_path("/App/View.swift"), "App/View.swift");
        assert_eq!(relative_path("App/View.swift"), "App/View.swift");
    }

    #[test]
    fn a_rule_appears_once_however_many_findings_use_it() {
        // A driver that lists a rule twice shows the same guidance twice, and
        // some readers reject the document outright.
        let mut report = sample();
        let extra = report.findings[0].clone();
        report.findings.push(extra);
        let sarif = render(&report);
        let ids: Vec<&str> = sarif["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .expect("rules")
            .iter()
            .filter_map(|r| r["id"].as_str())
            .collect();
        let unique: std::collections::BTreeSet<&&str> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "a rule was listed twice: {ids:?}");
    }

    #[test]
    fn every_result_names_a_rule_the_driver_describes() {
        // A result pointing at a rule the driver never declared shows up with
        // no title and no guidance.
        let sarif = render(&sample());
        let declared: std::collections::BTreeSet<String> = sarif["runs"][0]["tool"]["driver"]
            ["rules"]
            .as_array()
            .expect("rules")
            .iter()
            .filter_map(|r| r["id"].as_str().map(str::to_string))
            .collect();
        for result in results(&sarif) {
            let id = result["ruleId"].as_str().expect("a rule id");
            assert!(declared.contains(id), "{id} is used and never declared");
        }
    }

    #[test]
    fn the_rule_id_is_the_tracking_id_not_the_finding_id() {
        // Code Scanning follows an alert across pushes by ruleId. A finding id
        // changes every run, so using it would close and reopen every alert on
        // every push.
        let report = sample();
        let sarif = render(&report);
        let first = report
            .findings
            .iter()
            .find(|f| !f.overridden)
            .expect("a finding");
        assert_eq!(results(&sarif)[0]["ruleId"], first.rule_id.to_string());
        assert_ne!(
            results(&sarif)[0]["ruleId"],
            first.finding_id.to_string(),
            "the finding id was used as the tracking id"
        );
    }

    #[test]
    fn a_finding_with_no_line_still_reports() {
        // A rule about a missing file has nothing to point at. Dropping the
        // finding would hide it, so the location goes and the result stays.
        let mut report = sample();
        let id = report.findings[0].rule_id.to_string();
        report.findings[0].location = None;
        let sarif = render(&report);
        let found = results(&sarif)
            .iter()
            .find(|r| r["ruleId"] == id)
            .expect("the finding survived");
        assert!(found.get("locations").is_none());
    }

    #[test]
    fn a_zero_line_is_dropped_rather_than_sent() {
        // SARIF counts lines from one. A zero makes GitHub drop the location
        // without saying anything, so the finding lands nowhere.
        let mut report = sample();
        let id = report.findings[0].rule_id.to_string();
        if let Some(location) = report.findings[0].location.as_mut() {
            location.line_start = Some(0);
            location.line_end = Some(0);
        } else {
            report.findings[0].location = Some(crate::schema::Location {
                file: "App/View.swift".to_string(),
                line_start: Some(0),
                line_end: Some(0),
                snippet: None,
            });
        }
        let sarif = render(&report);
        let found = results(&sarif)
            .iter()
            .find(|r| r["ruleId"] == id)
            .expect("the finding survived");
        let physical = &found["locations"][0]["physicalLocation"];
        assert!(physical.get("region").is_none(), "a zero line was sent");
        assert!(physical["artifactLocation"]["uri"].is_string());
    }

    #[test]
    fn the_policy_and_the_remediation_reach_the_reader() {
        // A finding that names no source is an assertion. The url is how
        // somebody checks the rule against the policy themselves, and the
        // remediation is what they do about it.
        let sarif = render(&sample());
        let rules = sarif["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .expect("rules");
        assert!(!rules.is_empty(), "the fixture raised nothing");
        for rule in rules {
            assert!(rule["helpUri"].as_str().is_some_and(|u| !u.is_empty()));
            assert!(rule["properties"]["policySection"].is_string());
            assert!(rule["help"]["text"].as_str().is_some_and(|t| !t.is_empty()));
        }
    }

    #[test]
    fn an_empty_report_is_valid_sarif_rather_than_nothing() {
        // A clean run must still upload. Without a run object Code Scanning
        // keeps yesterday's alerts open forever.
        let mut report = sample();
        report.findings.clear();
        let sarif = render(&report);
        assert_eq!(sarif["version"], SCHEMA_VERSION);
        assert_eq!(results(&sarif).len(), 0);
        assert_eq!(
            sarif["runs"][0]["tool"]["driver"]["rules"]
                .as_array()
                .map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn the_run_names_the_database_that_produced_it() {
        let report = sample();
        let sarif = render(&report);
        assert_eq!(
            sarif["runs"][0]["properties"]["ruleDatabaseVersion"],
            report.rule_database_version
        );
    }
}
