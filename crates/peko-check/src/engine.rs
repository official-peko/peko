//! The mechanical check executor.
//!
//! The executor walks the validated mechanical rules, dispatches each check by
//! type, and collects findings. It never calls a language model.
//!
//! # When a rule is skipped
//!
//! A check needs its input. A rule that reads `Info.plist` cannot run against
//! a project that holds none. Such a rule is skipped, not failed, and the
//! report states the skipped count. A rule counts as checked when at least one
//! of its checks ran.

use crate::config::{OverrideStatus, PekoConfig};
use crate::discovery::build_glob_set;
use crate::error::Result;
use crate::finding::{Finding, Location};
use crate::knowledge::KnowledgeBase;
use crate::matcher::{evaluate, is_unresolved_reference, setting_to_value, MatchOutcome};
use crate::project::Project;
use crate::source::SourceFile;
use globset::GlobSet;
use peko_parse::{display_value, Document};
use peko_rules::{
    ConfigFile, ManifestFile, MechanicalCheck, Precondition, PrivacyManifestRequirement, Rule,
    RuleId, Severity,
};
use regex::{Regex, RegexBuilder};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

/// The default cap on matches reported by one source scan. A rule can lower it
/// with `max_matches`. The cap stops one bad pattern from filling a report.
pub const DEFAULT_MAX_MATCHES: usize = 25;

/// Lines of context kept on each side of a source match.
pub const SNIPPET_CONTEXT: usize = 1;

/// The result of one mechanical run.
#[derive(Debug, Clone, Default)]
pub struct MechanicalOutcome {
    pub findings: Vec<Finding>,
    /// Rules that ran at least one check.
    pub rules_checked: usize,
    /// Which rules ran at least one check.
    ///
    /// A rule that never ran carries no evidence from this project, and
    /// "it raised nothing" says nothing about it. Promotion has to tell the
    /// two apart.
    pub evaluated: BTreeSet<RuleId>,
    /// Rules whose input was absent from the project.
    pub rules_skipped: usize,
    /// Rules that do not apply to this project, because a precondition does
    /// not hold. A Kids Category rule against an adult app lands here.
    pub rules_not_applicable: usize,
    /// Facts that a rule needs and that `.pekorc.json` does not declare. Each
    /// one is a rule that stays unevaluated until the developer answers.
    pub undeclared_facts: Vec<String>,
    /// Facts the run assumed, because the vocabulary carries a default and
    /// nobody answered. A finding that rests on a guess has to say so.
    pub assumed_facts: Vec<String>,
    /// Direct dependencies read out of the lockfiles.
    pub checked_dependencies: usize,
    /// Direct dependencies that carry a flag.
    pub flagged_dependencies: usize,
    /// Direct dependencies missing from the knowledge base.
    pub unknown_dependencies: Vec<String>,
    /// Non-fatal problems, for example a rule with an unusable pattern.
    pub warnings: Vec<String>,
}

/// A cache of compiled globs and regular expressions.
///
/// Rules reuse the same glob sets and patterns across a run. Compiling once
/// keeps a full-codebase scan inside the 30 second target.
#[derive(Default)]
struct Compiled {
    globs: HashMap<String, GlobSet>,
    regexes: HashMap<String, Regex>,
}

impl Compiled {
    fn glob_set(&mut self, patterns: &[String]) -> Result<&GlobSet> {
        let key = patterns.join("\u{1}");
        if !self.globs.contains_key(&key) {
            let set = build_glob_set(patterns)?;
            self.globs.insert(key.clone(), set);
        }
        Ok(&self.globs[&key])
    }

    fn regex(
        &mut self,
        pattern: &str,
        case_insensitive: bool,
    ) -> std::result::Result<&Regex, regex::Error> {
        let key = format!("{}\u{1}{pattern}", u8::from(case_insensitive));
        if !self.regexes.contains_key(&key) {
            let regex = RegexBuilder::new(pattern)
                .case_insensitive(case_insensitive)
                .build()?;
            self.regexes.insert(key.clone(), regex);
        }
        Ok(&self.regexes[&key])
    }
}

/// Run every mechanical rule against a project.
///
/// Pass a knowledge base to enable the `dependency_flag` checks. Without one,
/// those checks are skipped and the report says so.
pub fn run(
    project: &Project,
    rules: &[&Rule],
    config: &PekoConfig,
    knowledge: Option<&KnowledgeBase>,
) -> Result<MechanicalOutcome> {
    let mut outcome = MechanicalOutcome::default();
    let mut compiled = Compiled::default();
    let mut unknown: BTreeSet<String> = BTreeSet::new();
    let mut flagged: BTreeSet<String> = BTreeSet::new();
    let mut undeclared: BTreeSet<String> = BTreeSet::new();

    outcome.checked_dependencies = project.dependencies.len();

    for rule in rules {
        if !rule.is_mechanical() || !rule.applies_to_platform(project.platform) {
            continue;
        }

        // A rule runs only where it applies. An undecided precondition leaves
        // the rule alone, so an unknown context never becomes a finding.
        match preconditions_hold(rule, project, config, &mut compiled) {
            Applicability::Applies => {}
            Applicability::DoesNotApply => {
                outcome.rules_not_applicable += 1;
                continue;
            }
            Applicability::Undecided(fact) => {
                outcome.rules_not_applicable += 1;
                undeclared.insert(fact);
                continue;
            }
        }

        let mut any_check_ran = false;
        for check in &rule.detection.mechanical_checks {
            let context = CheckContext {
                rule,
                project,
                knowledge,
                compiled: &mut compiled,
                unknown: &mut unknown,
                flagged: &mut flagged,
                warnings: &mut outcome.warnings,
            };
            any_check_ran |= run_check(check, context, &mut outcome.findings)?;
        }
        if any_check_ran {
            outcome.rules_checked += 1;
            outcome.evaluated.insert(rule.rule_id);
        } else {
            outcome.rules_skipped += 1;
        }
    }

    outcome.unknown_dependencies = unknown.into_iter().collect();
    outcome.flagged_dependencies = flagged.len();
    outcome.undeclared_facts = undeclared.into_iter().collect();
    // Only the assumptions a rule here actually rests on. The vocabulary sets
    // a default for 34 facts, and naming all of them buries the two or three
    // that changed an answer for this project.
    let consulted: BTreeSet<String> = rules
        .iter()
        .flat_map(|rule| fact_keys(&rule.detection.applies_when))
        .collect();
    outcome.assumed_facts = project
        .assumed_facts
        .iter()
        .filter(|name| consulted.contains(*name))
        .cloned()
        .collect();

    apply_overrides(&mut outcome.findings, rules, config);
    outcome.findings.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then(a.rule_id.cmp(&b.rule_id))
            .then_with(|| location_key(a).cmp(&location_key(b)))
    });
    Ok(outcome)
}

fn location_key(finding: &Finding) -> (String, usize) {
    match &finding.location {
        Some(location) => (
            location.file.to_string_lossy().into_owned(),
            location.line_start.unwrap_or(0),
        ),
        None => (String::new(), 0),
    }
}

/// Whether a rule applies to a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applicability {
    Applies,
    DoesNotApply,
    /// A fact decides it, and `.pekorc.json` does not declare that fact.
    Undecided(String),
}

/// Decide whether a rule applies to a project.
///
/// The mechanical checker calls the private form with its own cache. The audit
/// tier has no cache, so it calls this. Both must answer the same, because a
/// precondition that holds for a mechanical rule and not for an interpretive
/// one would be a second, silent rule language.
pub fn rule_applies(rule: &Rule, project: &Project, config: &PekoConfig) -> Applicability {
    let mut compiled = Compiled::default();
    preconditions_hold(rule, project, config, &mut compiled)
}

/// Decide whether every precondition of a rule holds.
fn preconditions_hold(
    rule: &Rule,
    project: &Project,
    config: &PekoConfig,
    compiled: &mut Compiled,
) -> Applicability {
    // Every condition on a rule is joined with and.
    for condition in &rule.detection.applies_when {
        match evaluate_precondition(condition, rule, project, config, compiled) {
            Applicability::Applies => {}
            other => return other,
        }
    }
    Applicability::Applies
}

/// Every fact name a set of conditions reads, including nested ones.
fn fact_keys(conditions: &[Precondition]) -> Vec<String> {
    let mut out = Vec::new();
    for condition in conditions {
        if let Precondition::AnyOf { conditions: inner } = condition {
            out.extend(fact_keys(inner));
        } else if let Some(key) = condition.fact_key() {
            out.push(key.to_string());
        }
    }
    out
}

/// Decide one condition.
fn evaluate_precondition(
    condition: &Precondition,
    rule: &Rule,
    project: &Project,
    config: &PekoConfig,
    compiled: &mut Compiled,
) -> Applicability {
    let holds = match condition {
        Precondition::CheckPasses { check } => {
            match run_precondition_check(check, rule, project, compiled) {
                Some(found) => !found,
                None => return Applicability::Undecided(describe_check(check)),
            }
        }
        Precondition::CheckFails { check } => {
            match run_precondition_check(check, rule, project, compiled) {
                Some(found) => found,
                None => return Applicability::Undecided(describe_check(check)),
            }
        }
        Precondition::AnyOf { conditions } => {
            // One branch that holds settles it. A branch that cannot be
            // decided only matters when no branch holds, because then the
            // answer turns on the fact nobody declared.
            let mut undecided: Option<String> = None;
            for inner in conditions {
                match evaluate_precondition(inner, rule, project, config, compiled) {
                    Applicability::Applies => return Applicability::Applies,
                    Applicability::Undecided(reason) => undecided = Some(reason),
                    Applicability::DoesNotApply => {}
                }
            }
            return match undecided {
                Some(reason) => Applicability::Undecided(reason),
                None => Applicability::DoesNotApply,
            };
        }
        Precondition::FactEquals { key, value } => match project.fact(key, config) {
            Some(declared) => declared == value,
            None => return Applicability::Undecided(key.clone()),
        },
        Precondition::FactContains { key, value } => match project.fact(key, config) {
            Some(serde_json::Value::Array(items)) => items.contains(value),
            Some(other) => other == value,
            None => return Applicability::Undecided(key.clone()),
        },
        Precondition::FactPresent { key } => project.fact(key, config).is_some(),
    };
    if holds {
        Applicability::Applies
    } else {
        Applicability::DoesNotApply
    }
}

/// Run one check for a precondition, and report whether it found anything.
///
/// `None` means the check had no input, so the precondition stays undecided.
fn run_precondition_check(
    check: &MechanicalCheck,
    rule: &Rule,
    project: &Project,
    compiled: &mut Compiled,
) -> Option<bool> {
    let mut findings = Vec::new();
    let mut warnings = Vec::new();
    let mut unknown = BTreeSet::new();
    let mut flagged = BTreeSet::new();
    let context = CheckContext {
        rule,
        project,
        knowledge: None,
        compiled,
        unknown: &mut unknown,
        flagged: &mut flagged,
        warnings: &mut warnings,
    };
    match run_check(check, context, &mut findings) {
        Ok(true) => Some(!findings.is_empty()),
        // The check had no input to read, or it failed. Either way the
        // precondition cannot be decided.
        Ok(false) | Err(_) => None,
    }
}

fn describe_check(check: &MechanicalCheck) -> String {
    format!("a {} check", check.check_type())
}

struct CheckContext<'a> {
    rule: &'a Rule,
    project: &'a Project,
    knowledge: Option<&'a KnowledgeBase>,
    compiled: &'a mut Compiled,
    unknown: &'a mut BTreeSet<String>,
    flagged: &'a mut BTreeSet<String>,
    warnings: &'a mut Vec<String>,
}

/// Run one check. Returns true when the check had its input and ran.
// The function is one dispatch table, so its length follows the check list.
#[allow(clippy::too_many_lines)]
fn run_check(
    check: &MechanicalCheck,
    context: CheckContext<'_>,
    findings: &mut Vec<Finding>,
) -> Result<bool> {
    let CheckContext {
        rule,
        project,
        knowledge,
        compiled,
        unknown,
        flagged,
        warnings,
    } = context;

    match check {
        MechanicalCheck::ManifestKeyPresent { file, key } => {
            Ok(check_key_present(rule, project, *file, key, findings))
        }
        MechanicalCheck::ManifestKeyAbsent { file, key } => {
            Ok(check_key_absent(rule, project, *file, key, findings))
        }
        MechanicalCheck::ManifestKeyValue {
            file,
            key,
            expect,
            required,
        } => Ok(check_key_value(
            rule, project, *file, key, expect, *required, findings,
        )),
        MechanicalCheck::ManifestKeyContains { file, key, value } => Ok(check_key_contains(
            rule, project, *file, key, value, findings,
        )),
        MechanicalCheck::EntitlementPresent { key } => {
            Ok(check_entitlement(rule, project, key, true, findings))
        }
        MechanicalCheck::EntitlementAbsent { key } => {
            Ok(check_entitlement(rule, project, key, false, findings))
        }
        MechanicalCheck::RegexSource {
            pattern,
            expect_present,
            include,
            exclude,
            case_insensitive,
            max_matches,
        } => check_source_pattern(
            &SourceScan {
                rule,
                project,
                pattern,
                include,
                exclude,
                case_insensitive: *case_insensitive,
                max_matches: *max_matches,
                require_any_import: &[],
                expect_present: *expect_present,
                check_type: "regex_source",
            },
            compiled,
            warnings,
            findings,
        ),
        MechanicalCheck::BundleEntry {
            pattern,
            expect_present,
        } => Ok(check_bundle_entry(
            rule,
            project,
            pattern,
            *expect_present,
            findings,
        )),
        MechanicalCheck::ApiUsage {
            symbol,
            expect_present,
            require_any_import,
            include,
            exclude,
            max_matches,
        } => check_source_pattern(
            &SourceScan {
                rule,
                project,
                pattern: symbol,
                include,
                exclude,
                case_insensitive: false,
                max_matches: *max_matches,
                require_any_import,
                expect_present: *expect_present,
                check_type: "api_usage",
            },
            compiled,
            warnings,
            findings,
        ),
        MechanicalCheck::DependencyFlag {
            flag_type,
            ecosystems,
        } => match knowledge {
            // A dependency check needs the knowledge base. Without one the
            // rule is skipped.
            None => Ok(false),
            Some(base) => Ok(check_dependency_flag(
                rule, project, *flag_type, ecosystems, base, unknown, flagged, findings,
            )),
        },
        MechanicalCheck::PrivacyManifest { requirement } => Ok(check_privacy_manifest(
            rule,
            project,
            compiled,
            requirement,
            findings,
        )),
        MechanicalCheck::ConfigValue {
            file,
            setting,
            expect,
            required,
        } => Ok(check_config_value(
            rule, project, *file, setting, expect, *required, warnings, findings,
        )),
    }
}

/// Point a manifest finding at a line.
///
/// A manifest parser builds a tree without positions. The finding names the
/// key and the value it objects to, and a text search names the line. The
/// search tries the most specific form first, because a bare value such as
/// `true` appears many times in one manifest.
///
/// The order is:
///
/// 1. The XML attribute form, `key="value"`.
/// 2. The property list form, `<key>key</key>`.
/// 3. The value alone, when it is long enough to be distinctive.
/// 4. The key alone.
///
/// A miss returns a location that names the file alone, which is still enough
/// to act on.
fn locate_in_document(document: &Document, value: Option<&str>, key: &str) -> Location {
    /// A value shorter than this appears too often to name a line on its own.
    const DISTINCTIVE_VALUE_LENGTH: usize = 12;

    let leaf = key
        .rsplit(['.', '@'])
        .find(|part| !part.is_empty() && !part.starts_with('['))
        .unwrap_or(key);

    let mut candidates: Vec<String> = Vec::new();
    if let Some(value) = value {
        candidates.push(format!("{leaf}=\"{value}\""));
        candidates.push(format!("<key>{leaf}</key>"));
        if value.len() >= DISTINCTIVE_VALUE_LENGTH {
            candidates.push(value.to_string());
        }
    } else {
        candidates.push(format!("<key>{leaf}</key>"));
    }
    candidates.push(leaf.to_string());

    let line = candidates
        .iter()
        .find_map(|candidate| document.line_of(candidate));

    match line {
        Some(line) => Location::line(
            document.path(),
            line,
            document.line_text(line).unwrap_or_default().trim(),
        ),
        None => Location::file(document.path()),
    }
}

fn manifest_label(file: ManifestFile) -> &'static str {
    match file {
        ManifestFile::InfoPlist => "Info.plist",
        ManifestFile::AndroidManifest => "AndroidManifest.xml",
        ManifestFile::PrivacyManifest => "PrivacyInfo.xcprivacy",
        ManifestFile::Entitlements => "the entitlements file",
    }
}

/// A key must be present. One project can hold several manifests, for example
/// an app and its extensions. The requirement is met when any manifest holds
/// the key.
fn check_key_present(
    rule: &Rule,
    project: &Project,
    file: ManifestFile,
    key: &str,
    findings: &mut Vec<Finding>,
) -> bool {
    let documents = project.manifests_for_key(file, key);
    if documents.is_empty() {
        return false;
    }
    let found = documents
        .iter()
        .any(|document| document.lookup(key).is_ok_and(|values| !values.is_empty()));
    if !found {
        let location = Location::file(documents[0].path());
        findings.push(Finding::mechanical(
            rule.rule_id,
            rule.severity,
            "manifest_key_present",
            format!("{key} is missing from {}", manifest_label(file)),
            Some(location),
        ));
    }
    true
}

/// A key must be absent. Every manifest that holds it produces a finding.
fn check_key_absent(
    rule: &Rule,
    project: &Project,
    file: ManifestFile,
    key: &str,
    findings: &mut Vec<Finding>,
) -> bool {
    let documents = project.manifests_for_key(file, key);
    if documents.is_empty() {
        return false;
    }
    for document in documents {
        let Ok(values) = document.lookup(key) else {
            continue;
        };
        if let Some(value) = values.first() {
            let rendered = display_value(value);
            findings.push(Finding::mechanical(
                rule.rule_id,
                rule.severity,
                "manifest_key_absent",
                format!(
                    "{key} is present in {} with value {rendered}",
                    manifest_label(file)
                ),
                Some(locate_in_document(document, Some(&rendered), key)),
            ));
        }
    }
    true
}

fn check_key_value(
    rule: &Rule,
    project: &Project,
    file: ManifestFile,
    key: &str,
    expect: &peko_rules::ValueMatcher,
    required: bool,
    findings: &mut Vec<Finding>,
) -> bool {
    let documents = project.manifests_for_key(file, key);
    if documents.is_empty() {
        return false;
    }

    let mut seen_anywhere = false;
    for document in documents {
        let Ok(values) = document.lookup(key) else {
            continue;
        };
        if values.is_empty() {
            continue;
        }
        seen_anywhere = true;
        for value in values {
            if let Some(reason) = evaluate(expect, value).failure() {
                let rendered = display_value(value);
                findings.push(Finding::mechanical(
                    rule.rule_id,
                    rule.severity,
                    "manifest_key_value",
                    format!("{key} in {}: {reason}", manifest_label(file)),
                    Some(locate_in_document(document, Some(&rendered), key)),
                ));
            }
        }
    }

    if !seen_anywhere && required {
        findings.push(Finding::mechanical(
            rule.rule_id,
            rule.severity,
            "manifest_key_value",
            format!("{key} is missing from {}", manifest_label(file)),
            Some(Location::file(documents[0].path())),
        ));
    }
    true
}

/// A list must hold a value.
///
/// The check reports once when no manifest holds it, which suits a rule that
/// says "the app must declare this permission".
fn check_key_contains(
    rule: &Rule,
    project: &Project,
    file: ManifestFile,
    key: &str,
    wanted: &Value,
    findings: &mut Vec<Finding>,
) -> bool {
    let documents = project.manifests_for_key(file, key);
    if documents.is_empty() {
        return false;
    }

    let found = documents.iter().any(|document| {
        document
            .lookup(key)
            .unwrap_or_default()
            .into_iter()
            .any(|value| value == wanted || value.as_str() == wanted.as_str())
    });

    if !found {
        findings.push(Finding::mechanical(
            rule.rule_id,
            rule.severity,
            "manifest_key_contains",
            format!(
                "{} does not list {} at {key}",
                manifest_label(file),
                display_value(wanted)
            ),
            Some(Location::file(documents[0].path())),
        ));
    }
    true
}

/// Entitlement keys contain dots, so they read as literal top level keys, not
/// as key paths.
/// Whether the built app holds an entry matching a glob.
///
/// Returns false when there is no bundle, which counts the rule as skipped.
/// A rule that asks what shipped cannot be answered from a repository, and
/// answering it anyway is how a report claims more than it knows. Every other
/// place in this file takes the same position.
fn check_bundle_entry(
    rule: &Rule,
    project: &Project,
    pattern: &str,
    expect_present: bool,
    findings: &mut Vec<Finding>,
) -> bool {
    let Some(bundle) = project.bundle.as_ref() else {
        return false;
    };
    let Ok(glob) = globset::Glob::new(pattern) else {
        return false;
    };
    let matcher = glob.compile_matcher();
    let found = bundle
        .entries
        .iter()
        .any(|entry| matcher.is_match(entry.as_str()));

    if found != expect_present {
        findings.push(Finding::mechanical(
            rule.rule_id,
            rule.severity,
            "bundle_entry",
            if expect_present {
                format!("The built app holds no entry matching {pattern}")
            } else {
                format!("The built app holds an entry matching {pattern}")
            },
            None,
        ));
    }
    true
}

fn check_entitlement(
    rule: &Rule,
    project: &Project,
    key: &str,
    must_be_present: bool,
    findings: &mut Vec<Finding>,
) -> bool {
    let documents = &project.entitlements;
    if documents.is_empty() {
        return false;
    }

    if must_be_present {
        let found = documents
            .iter()
            .any(|document| document.get_literal(key).is_some());
        if !found {
            findings.push(Finding::mechanical(
                rule.rule_id,
                rule.severity,
                "entitlement_present",
                format!("The entitlement {key} is not declared"),
                Some(Location::file(documents[0].path())),
            ));
        }
    } else {
        for document in documents {
            if document.get_literal(key).is_some() {
                findings.push(Finding::mechanical(
                    rule.rule_id,
                    rule.severity,
                    "entitlement_absent",
                    format!("The entitlement {key} is declared"),
                    Some(locate_in_document(document, Some(key), key)),
                ));
            }
        }
    }
    true
}

/// One source scan, as a single argument.
struct SourceScan<'a> {
    rule: &'a Rule,
    project: &'a Project,
    pattern: &'a str,
    include: &'a [String],
    exclude: &'a [String],
    case_insensitive: bool,
    max_matches: Option<usize>,
    require_any_import: &'a [String],
    /// False reports every match, which suits a banned API. True reports once
    /// when nothing matches, which suits a required call.
    expect_present: bool,
    check_type: &'a str,
}

/// Scan source files with a pattern.
///
/// `require_any_import` cuts false positives. When the list is not empty, a
/// file must also hold one of the listed import statements before a match
/// counts.
fn check_source_pattern(
    scan: &SourceScan<'_>,
    compiled: &mut Compiled,
    warnings: &mut Vec<String>,
    findings: &mut Vec<Finding>,
) -> Result<bool> {
    let &SourceScan {
        rule,
        project,
        pattern,
        include,
        exclude,
        case_insensitive,
        max_matches,
        require_any_import,
        expect_present,
        check_type,
    } = scan;

    // Only a check about calls treats a name inside a string as noise. A text
    // search is looking for text, and a placeholder string or a mining pool
    // name lives in exactly the literal this would skip.
    let reads_calls = check_type == "api_usage";

    if project.sources.is_empty() {
        return Ok(false);
    }

    let include_set = compiled.glob_set(include)?.clone();
    let exclude_set = compiled.glob_set(exclude)?.clone();
    let regex = match compiled.regex(pattern, case_insensitive) {
        Ok(regex) => regex.clone(),
        Err(error) => {
            warnings.push(format!(
                "rule {} holds an invalid pattern /{pattern}/: {error}",
                rule.rule_id
            ));
            return Ok(false);
        }
    };

    let limit = max_matches.unwrap_or(DEFAULT_MAX_MATCHES);
    let mut reported = 0usize;
    let mut scanned_any = false;

    for file in &project.sources {
        if !expect_present && reported >= limit {
            break;
        }
        if !include_set.is_match(file.relative()) {
            continue;
        }
        if !exclude.is_empty() && exclude_set.is_match(file.relative()) {
            continue;
        }
        if !require_any_import.is_empty() && !file.contains_any(require_any_import) {
            continue;
        }
        scanned_any = true;

        if expect_present {
            // One match anywhere satisfies the requirement.
            if regex.find_iter(file.text()).any(|found| {
                !(is_inside_comment(file.text(), found.start())
                    || (reads_calls && is_inside_data_string(file.text(), found.start())))
            }) {
                return Ok(true);
            }
            continue;
        }

        reported += report_matches(
            rule,
            file,
            &regex,
            check_type,
            limit - reported,
            check_type == "api_usage",
            reads_calls,
            findings,
        );
    }

    if expect_present {
        if !scanned_any {
            // No file could hold the call, so the check has no input.
            return Ok(false);
        }
        findings.push(Finding::mechanical(
            rule.rule_id,
            rule.severity,
            check_type,
            format!("no source file calls {pattern}, and this rule requires it"),
            None,
        ));
    }
    Ok(true)
}

/// True when the offset sits inside a string literal that is only data.
///
/// A banned API rule reads a name out of source, and a name inside a string is
/// weaker evidence than a name in code: a string does not link a symbol into
/// the binary, and Apple reads the binary. `FSNotes` carries a syntax
/// highlighter whose Objective-C keyword list holds `"UIWebView"` beside
/// `"WKWebView"` and thirty other class names, and the app calls neither.
///
/// The exception matters more than the rule. `NSClassFromString("UIWebView")`
/// is a string and it is also a call, so a string that follows a dynamic
/// lookup still counts.
fn is_inside_data_string(text: &str, offset: usize) -> bool {
    // A dynamic lookup turns the string back into a call.
    const LOOKUPS: [&str; 5] = [
        "NSClassFromString",
        "NSSelectorFromString",
        "Class.forName",
        "objc_getClass",
        "dlsym",
    ];
    let bytes = text.as_bytes();
    let line_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);

    let mut quote: Option<u8> = None;
    let mut index = line_start;
    while index < offset {
        let byte = bytes[index];
        match quote {
            Some(open) => {
                if byte == b'\\' {
                    index += 2;
                    continue;
                }
                if byte == open {
                    quote = None;
                }
            }
            None if byte == b'"' || byte == b'\'' => quote = Some(byte),
            None => {}
        }
        index += 1;
    }
    if quote.is_none() {
        return false;
    }

    let window_start = line_start.max(offset.saturating_sub(120));
    let window = &text[window_start..offset];
    !LOOKUPS.iter().any(|name| window.contains(name))
}

/// True when the offset sits inside a comment.
///
/// A validation run against published apps found two false positives from
/// comment text alone. Wikipedia holds the words "due to problems in
/// `UIWebView`" in a note, and the `WordPress` app holds a doc comment that
/// names `UIWebView`. Neither app calls the API.
///
/// The reader handles a line comment and a block comment. It tracks a quoted
/// string, so a `//` inside a URL literal does not open a comment.
fn is_inside_comment(text: &str, offset: usize) -> bool {
    let bytes = text.as_bytes();
    let line_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);

    // A line comment that opens before the offset.
    let mut quote: Option<u8> = None;
    let mut index = line_start;
    while index < offset {
        let byte = bytes[index];
        match quote {
            Some(open) => {
                if byte == b'\\' {
                    index += 2;
                    continue;
                }
                if byte == open {
                    quote = None;
                }
            }
            None => {
                if byte == b'"' || byte == b'\'' {
                    quote = Some(byte);
                } else if byte == b'#'
                    || (byte == b'/' && index + 1 < offset && bytes[index + 1] == b'/')
                {
                    return true;
                }
            }
        }
        index += 1;
    }

    // A block comment that opens before the offset and does not close.
    let open = text[..offset].rfind("/*");
    let close = text[..offset].rfind("*/");
    match (open, close) {
        (Some(open_at), Some(close_at)) => open_at > close_at,
        (Some(_), None) => true,
        _ => false,
    }
}

/// A declaration is not a use. An `api_usage` check therefore skips a match
/// that sits on an import line, so that one import plus one call reports one
/// finding, not two.
fn is_declaration_line(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("import ")
        || trimmed.starts_with("#import")
        || trimmed.starts_with("@import")
        || trimmed.starts_with("#include")
        || trimmed.starts_with("using ")
}

#[allow(clippy::too_many_arguments)]
fn report_matches(
    rule: &Rule,
    file: &SourceFile,
    regex: &Regex,
    check_type: &str,
    remaining: usize,
    skip_declarations: bool,
    // True when the check asks whether the code calls something, so a name
    // inside a string does not count. A text search wants the strings.
    reads_calls: bool,
    findings: &mut Vec<Finding>,
) -> usize {
    let mut count = 0usize;
    for capture in regex.find_iter(file.text()) {
        if count >= remaining {
            break;
        }
        // A comment is prose about the code, not the code.
        if is_inside_comment(file.text(), capture.start())
            || (reads_calls && is_inside_data_string(file.text(), capture.start()))
        {
            continue;
        }
        let line = file.line_of(capture.start());
        if skip_declarations && is_declaration_line(file.line_text(line)) {
            continue;
        }
        findings.push(Finding::mechanical(
            rule.rule_id,
            rule.severity,
            check_type,
            format!("{} matches at line {line}", capture.as_str().trim()),
            Some(Location::line(
                file.relative(),
                line,
                file.snippet(line, SNIPPET_CONTEXT),
            )),
        ));
        count += 1;
    }
    count
}

/// Check a build setting.
///
/// Two limits keep this honest against a real repository.
///
/// A multi-module Android project holds one build script per module, and only
/// the module that applies the application plugin becomes the app. A library
/// module sets its own target level, and that level says nothing about the app
/// that ships.
///
/// A build script often names a variable instead of a value. This checker does
/// not evaluate a build script, so an unresolved name decides nothing.
#[allow(clippy::too_many_arguments)]
fn check_config_value(
    rule: &Rule,
    project: &Project,
    file: ConfigFile,
    setting: &str,
    expect: &peko_rules::ValueMatcher,
    required: bool,
    warnings: &mut Vec<String>,
    findings: &mut Vec<Finding>,
) -> bool {
    let all = project.configs(file);
    let documents: Vec<&crate::project::ConfigDocument> = match file {
        ConfigFile::BuildGradle => all.iter().filter(|entry| entry.is_application).collect(),
        ConfigFile::XcodeProject => all.iter().collect(),
    };
    if documents.is_empty() {
        return false;
    }

    let mut seen_anywhere = false;
    for document in &documents {
        for value in document.settings.get(setting) {
            if is_unresolved_reference(&value.value) {
                warnings.push(format!(
                    "{} in {}: {setting} names {}, which this checker cannot resolve",
                    rule.rule_id,
                    document.relative.display(),
                    value.value
                ));
                continue;
            }
            seen_anywhere = true;
            let parsed = setting_to_value(&value.value);
            match evaluate(expect, &parsed) {
                MatchOutcome::Pass => {}
                MatchOutcome::Fail(reason) => findings.push(Finding::mechanical(
                    rule.rule_id,
                    rule.severity,
                    "config_value",
                    format!("{setting}: {reason}"),
                    Some(Location::line(
                        &document.relative,
                        value.line,
                        value.value.clone(),
                    )),
                )),
                MatchOutcome::NotEvaluable(reason) => warnings.push(format!(
                    "{} in {}: {reason}",
                    rule.rule_id,
                    document.relative.display()
                )),
            }
        }
    }

    if !seen_anywhere && required {
        findings.push(Finding::mechanical(
            rule.rule_id,
            rule.severity,
            "config_value",
            format!("{setting} is not set"),
            Some(Location::file(&documents[0].relative)),
        ));
    }
    true
}

/// Check one requirement inside `PrivacyInfo.xcprivacy`.
fn check_privacy_manifest(
    rule: &Rule,
    project: &Project,
    compiled: &mut Compiled,
    requirement: &PrivacyManifestRequirement,
    findings: &mut Vec<Finding>,
) -> bool {
    match requirement {
        PrivacyManifestRequirement::FilePresent => {
            // This check needs no privacy manifest to run. A missing file is
            // the finding.
            if project.privacy_manifests.is_empty() {
                findings.push(Finding::mechanical(
                    rule.rule_id,
                    rule.severity,
                    "privacy_manifest",
                    "The project holds no PrivacyInfo.xcprivacy file".to_string(),
                    None,
                ));
            }
            true
        }
        PrivacyManifestRequirement::TrackingFlagDeclared => {
            let Some(document) = project.privacy_manifests.first() else {
                return false;
            };
            if !document.exists("NSPrivacyTracking").unwrap_or(false) {
                findings.push(Finding::mechanical(
                    rule.rule_id,
                    rule.severity,
                    "privacy_manifest",
                    "NSPrivacyTracking is not declared in PrivacyInfo.xcprivacy".to_string(),
                    Some(Location::file(document.path())),
                ));
            }
            true
        }
        PrivacyManifestRequirement::CollectedDataDeclared { data_type } => {
            let Some(document) = project.privacy_manifests.first() else {
                return false;
            };
            let declared = document
                .lookup("NSPrivacyCollectedDataTypes.NSPrivacyCollectedDataType")
                .unwrap_or_default()
                .into_iter()
                .filter_map(Value::as_str)
                .any(|value| value == data_type);
            if !declared {
                findings.push(Finding::mechanical(
                    rule.rule_id,
                    rule.severity,
                    "privacy_manifest",
                    format!("{data_type} is not listed in NSPrivacyCollectedDataTypes"),
                    Some(Location::file(document.path())),
                ));
            }
            true
        }
        PrivacyManifestRequirement::ApiReasonDeclared {
            api_type,
            allowed_reasons,
            triggered_by,
        } => check_api_reason(
            rule,
            project,
            compiled,
            api_type,
            allowed_reasons,
            triggered_by,
            findings,
        ),
    }
}

/// A required reason API declaration is needed only when the code uses the
/// API. The check first looks for a trigger symbol in the source. Without a
/// trigger the rule does not apply.
// The function reads as one pass: find the trigger, find the target, read the
// manifest of that target, then compare the reason codes.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn check_api_reason(
    rule: &Rule,
    project: &Project,
    compiled: &mut Compiled,
    api_type: &str,
    allowed_reasons: &[String],
    triggered_by: &[String],
    findings: &mut Vec<Finding>,
) -> bool {
    let mut trigger: Option<(&SourceFile, usize)> = None;
    'outer: for file in &project.sources {
        for symbol in triggered_by {
            if let Ok(regex) = compiled.regex(symbol, false) {
                for found in regex.find_iter(file.text()) {
                    // The required reason APIs are calls, so a name inside a
                    // string does not put the symbol in the binary.
                    if is_inside_comment(file.text(), found.start())
                        || is_inside_data_string(file.text(), found.start())
                    {
                        continue;
                    }
                    trigger = Some((file, file.line_of(found.start())));
                    break 'outer;
                }
            }
        }
    }

    let Some((file, line)) = trigger else {
        return false;
    };

    let location = Some(Location::line(
        file.relative(),
        line,
        file.snippet(line, SNIPPET_CONTEXT),
    ));

    // A real app ships one privacy manifest per target. The source file that
    // triggered this check belongs to one target, so its manifest is the one
    // that must hold the declaration.
    let manifests = project.privacy_manifests_for(file.relative());
    let target_name = project
        .target_for(file.relative())
        .map_or_else(String::new, |target| format!(" of target {}", target.name));

    if manifests.is_empty() {
        findings.push(Finding::mechanical(
            rule.rule_id,
            rule.severity,
            "privacy_manifest",
            format!(
                "The code uses a {api_type} API, and the bundle{target_name} holds no \
                 PrivacyInfo.xcprivacy file"
            ),
            location,
        ));
        return true;
    }

    let mut declared_reasons: Vec<String> = Vec::new();
    let mut found_type = false;
    let mut document = manifests[0];
    for candidate in &manifests {
        let entries = candidate
            .lookup("NSPrivacyAccessedAPITypes")
            .unwrap_or_default();
        for entry in flatten_entries(&entries) {
            let entry_type = entry
                .get("NSPrivacyAccessedAPIType")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if entry_type != api_type {
                continue;
            }
            if !found_type {
                document = candidate;
            }
            found_type = true;
            if let Some(reasons) = entry
                .get("NSPrivacyAccessedAPITypeReasons")
                .and_then(Value::as_array)
            {
                declared_reasons
                    .extend(reasons.iter().filter_map(Value::as_str).map(str::to_string));
            }
        }
    }

    if !found_type {
        findings.push(Finding::mechanical(
            rule.rule_id,
            rule.severity,
            "privacy_manifest",
            format!(
                "The code uses a {api_type} API, and the privacy manifest{target_name} does \
                 not declare it"
            ),
            location,
        ));
        return true;
    }

    if declared_reasons.is_empty() {
        findings.push(Finding::mechanical(
            rule.rule_id,
            rule.severity,
            "privacy_manifest",
            format!("{api_type} is declared with no reason code"),
            Some(Location::file(document.path())),
        ));
        return true;
    }

    for reason in &declared_reasons {
        if !allowed_reasons.iter().any(|allowed| allowed == reason) {
            findings.push(Finding::mechanical(
                rule.rule_id,
                rule.severity,
                "privacy_manifest",
                format!(
                    "{api_type} declares reason {reason}, which is not one of [{}]",
                    allowed_reasons.join(", ")
                ),
                Some(Location::file(document.path())),
            ));
        }
    }
    true
}

fn flatten_entries<'a>(values: &[&'a Value]) -> Vec<&'a serde_json::Map<String, Value>> {
    let mut out = Vec::new();
    for value in values {
        match value {
            Value::Array(items) => {
                for item in items {
                    if let Some(map) = item.as_object() {
                        out.push(map);
                    }
                }
            }
            Value::Object(map) => out.push(map),
            _ => {}
        }
    }
    out
}

/// Look every direct dependency up in the knowledge base.
#[allow(clippy::too_many_arguments)]
fn check_dependency_flag(
    rule: &Rule,
    project: &Project,
    flag_type: peko_rules::DependencyFlagType,
    ecosystems: &[peko_rules::Ecosystem],
    knowledge: &KnowledgeBase,
    unknown: &mut BTreeSet<String>,
    flagged: &mut BTreeSet<String>,
    findings: &mut Vec<Finding>,
) -> bool {
    let relevant: Vec<&peko_parse::Dependency> = project
        .dependencies
        .iter()
        .filter(|dependency| ecosystems.contains(&dependency.ecosystem))
        .collect();
    if relevant.is_empty() {
        return false;
    }

    for dependency in relevant {
        let Some(entry) = knowledge.get(&dependency.package_id) else {
            unknown.insert(dependency.package_id.clone());
            continue;
        };
        for flag in &entry.compliance_flags {
            if flag.flag_type != flag_type || !flag.platform.applies_to(project.platform) {
                continue;
            }
            flagged.insert(dependency.package_id.clone());
            let location = dependency.line.map_or_else(
                || Location::file(&dependency.declared_in),
                |line| Location::line(&dependency.declared_in, line, dependency.name.clone()),
            );
            findings.push(Finding::mechanical(
                rule.rule_id,
                flag.severity.min(rule.severity),
                "dependency_flag",
                format!("{}: {}", dependency.package_id, flag.description),
                Some(location),
            ));
        }
    }
    true
}

/// Mark findings that `.pekorc.json` acknowledges.
///
/// A rule that is not overridable ignores an override. A user cannot silence a
/// critical mechanical rule.
fn apply_overrides(findings: &mut [Finding], rules: &[&Rule], config: &PekoConfig) {
    let overrides = config.override_map();
    let overridable: HashMap<RuleId, bool> = rules
        .iter()
        .map(|rule| (rule.rule_id, rule.overridable))
        .collect();

    for finding in findings.iter_mut() {
        let Some(entry) = overrides.get(&finding.rule_id) else {
            continue;
        };
        if !overridable.get(&finding.rule_id).copied().unwrap_or(false) {
            continue;
        }
        finding.overridden = true;
        finding.override_reason = Some(match entry.status {
            OverrideStatus::Acknowledged => entry.reason.clone(),
            OverrideStatus::NotApplicable => format!("not applicable: {}", entry.reason),
        });
    }
}

/// Count findings by severity, ignoring overridden findings.
pub fn severity_counts(findings: &[Finding]) -> HashMap<Severity, usize> {
    let mut counts = HashMap::new();
    for finding in findings {
        *counts.entry(finding.severity).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod bundle_tests {
    use super::*;
    use peko_parse::bundle::Bundle;
    use std::collections::BTreeMap;

    /// A bundle holding the entries a test names, and nothing else.
    fn bundle_with(entries: &[&str]) -> Bundle {
        Bundle {
            info_plist: BTreeMap::new(),
            privacy_manifest: None,
            frameworks: Vec::new(),
            abis: Vec::new(),
            entries: entries.iter().map(ToString::to_string).collect(),
            has_provisioning_profile: false,
        }
    }

    fn project() -> Project {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("the repository root")
            .join("fixtures/ios-compliant");
        let config = crate::PekoConfig::new(peko_rules::Platform::Ios);
        Project::load(&root, &config).expect("the fixture loads")
    }

    fn rule() -> Rule {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("the repository root")
            .join("rules");
        let database = peko_rules::RuleDatabase::load_from_dir(root).expect("the database loads");
        database
            .rules()
            .iter()
            .find(|rule| rule.rule_type == peko_rules::RuleType::Mechanical)
            .expect("a mechanical rule")
            .clone()
    }

    #[test]
    fn no_bundle_leaves_the_rule_undecided_rather_than_passing() {
        // The whole point. A rule that asks what shipped cannot be answered
        // from a repository, and a pass here is a claim nobody checked.
        let mut project = project();
        project.bundle = None;
        let mut findings = Vec::new();
        let decided = check_bundle_entry(
            &rule(),
            &project,
            "Payload/*.app/main.jsbundle",
            true,
            &mut findings,
        );
        assert!(!decided, "no bundle must leave the rule undecided");
        assert!(findings.is_empty(), "an undecided rule raises nothing");
    }

    #[test]
    fn an_entry_that_is_there_raises_nothing() {
        let mut project = project();
        project.bundle = Some(bundle_with(&[
            "Payload/App.app/Info.plist",
            "Payload/App.app/main.jsbundle",
        ]));
        let mut findings = Vec::new();
        assert!(check_bundle_entry(
            &rule(),
            &project,
            "Payload/*.app/main.jsbundle",
            true,
            &mut findings
        ));
        assert!(findings.is_empty());
    }

    #[test]
    fn an_entry_that_should_not_be_there_is_reported() {
        let mut project = project();
        project.bundle = Some(bundle_with(&["Payload/App.app/main.jsbundle"]));
        let mut findings = Vec::new();
        assert!(check_bundle_entry(
            &rule(),
            &project,
            "Payload/*.app/main.jsbundle",
            false,
            &mut findings
        ));
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn a_glob_that_matches_nothing_reports_the_absence() {
        let mut project = project();
        project.bundle = Some(bundle_with(&["Payload/App.app/Info.plist"]));
        let mut findings = Vec::new();
        assert!(check_bundle_entry(
            &rule(),
            &project,
            "base/lib/*/libflutter.so",
            true,
            &mut findings
        ));
        assert_eq!(findings.len(), 1);
    }
}
