//! The rule schema (specification section 4.4).

use crate::category::Category;
use crate::id::RuleId;
use crate::platform::Platform;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// How severe a finding from this rule is.
///
/// The declaration order defines the ordering: `Info < Warning < Error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    /// The next lower severity. `Info` is the floor.
    #[must_use]
    pub fn downgrade(self) -> Self {
        match self {
            Severity::Error => Severity::Warning,
            Severity::Warning | Severity::Info => Severity::Info,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Severity {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "info" => Ok(Severity::Info),
            "warning" | "warn" => Ok(Severity::Warning),
            "error" => Ok(Severity::Error),
            other => Err(format!("unknown severity {other:?}")),
        }
    }
}

/// Whether a rule is checked deterministically or by a language model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleType {
    /// Verified by parsing files or matching patterns.
    Mechanical,
    /// Requires judgment. Evaluated by a language model with a confidence score.
    Interpretive,
}

/// The human validation state of a rule (specification section 4.3).
///
/// The compiler writes `Candidate`. A human promotes a rule to `Validated`.
/// Only validated rules run against customer code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleStatus {
    /// Produced by the compiler. Not active.
    #[default]
    Candidate,
    /// Reviewed by a human. Active.
    Validated,
    /// Retired. Kept for the audit trail, never evaluated.
    Deprecated,
}

/// The kind of project data a check reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    Manifest,
    Entitlements,
    Source,
    Config,
    Dependency,
    Asset,
}

/// How likely one reading of an interpretive rule is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Likelihood {
    High,
    Medium,
    Low,
}

/// A manifest style file that the checker parses into a key-value document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestFile {
    /// `Info.plist`, binary or XML.
    InfoPlist,
    /// `AndroidManifest.xml`.
    AndroidManifest,
    /// `PrivacyInfo.xcprivacy`.
    PrivacyManifest,
    /// An `.entitlements` property list.
    Entitlements,
}

/// A build configuration file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFile {
    /// `project.pbxproj` inside an `.xcodeproj` bundle.
    XcodeProject,
    /// `build.gradle` or `build.gradle.kts`.
    BuildGradle,
}

/// A dependency ecosystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ecosystem {
    #[serde(alias = "cocoa_pods", alias = "pods")]
    Cocoapods,
    /// The alias list catches what a model writes when it reaches for the
    /// common name instead of the schema name. A live compile run answered
    /// `swiftpm`, and the whole request was lost to a parse error.
    #[serde(alias = "swiftpm", alias = "swift_pm", alias = "spm")]
    SwiftPackage,
    #[serde(alias = "maven")]
    Gradle,
    Npm,
}

impl Ecosystem {
    pub fn as_str(self) -> &'static str {
        match self {
            Ecosystem::Cocoapods => "cocoapods",
            Ecosystem::SwiftPackage => "swift-package",
            Ecosystem::Gradle => "gradle",
            Ecosystem::Npm => "npm",
        }
    }
}

impl fmt::Display for Ecosystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A compliance relevant behavior attached to a dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyFlagType {
    DataCollection,
    PrivateApi,
    Tracking,
    PermissionRequired,
    Deprecated,
    KnownRejection,
}

/// A test applied to a value read out of a manifest or a config file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "match", rename_all = "snake_case")]
pub enum ValueMatcher {
    /// The value equals `value` exactly.
    Equals { value: serde_json::Value },
    /// The value differs from `value`.
    NotEquals { value: serde_json::Value },
    /// The value is one of `values`.
    OneOf { values: Vec<serde_json::Value> },
    /// The string form of the value matches `pattern`.
    Regex { pattern: String },
    /// The string form of the value does not match `pattern`.
    NotRegex { pattern: String },
    /// The value is an integer at or above `value`.
    MinInt { value: i64 },
    /// The value is an integer at or below `value`.
    MaxInt { value: i64 },
    /// The value is a string with at least `min_length` non-space characters.
    NonEmptyString {
        #[serde(default = "default_min_length")]
        min_length: usize,
    },
}

fn default_min_length() -> usize {
    1
}

/// What a `privacy_manifest` check verifies inside `PrivacyInfo.xcprivacy`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "requirement", rename_all = "snake_case")]
pub enum PrivacyManifestRequirement {
    /// The file exists in the project.
    FilePresent,
    /// A required reason API category is declared, and the declared reason
    /// codes are inside `allowed_reasons`.
    ///
    /// The check fires only when one of `triggered_by` appears in the source.
    ApiReasonDeclared {
        /// The `NSPrivacyAccessedAPIType` value, for example
        /// `NSPrivacyAccessedAPICategoryUserDefaults`.
        api_type: String,
        /// The reason codes that Apple accepts for this category.
        #[serde(default)]
        allowed_reasons: Vec<String>,
        /// Source symbols that make this declaration necessary.
        #[serde(default)]
        triggered_by: Vec<String>,
    },
    /// `NSPrivacyTracking` is declared.
    TrackingFlagDeclared,
    /// A collected data type appears in `NSPrivacyCollectedDataTypes`.
    CollectedDataDeclared { data_type: String },
}

/// Default glob set for source scans.
fn default_source_include() -> Vec<String> {
    vec![
        "**/*.swift".into(),
        "**/*.m".into(),
        "**/*.mm".into(),
        "**/*.h".into(),
        "**/*.kt".into(),
        "**/*.java".into(),
    ]
}

/// One deterministic check. The serialized form is
/// `{"check_type": "...", "parameters": {...}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "check_type", content = "parameters", rename_all = "snake_case")]
pub enum MechanicalCheck {
    /// The key path exists in the manifest.
    ManifestKeyPresent { file: ManifestFile, key: String },
    /// The key path does not exist in the manifest.
    ManifestKeyAbsent { file: ManifestFile, key: String },
    /// The key path exists and its value satisfies the matcher.
    ManifestKeyValue {
        file: ManifestFile,
        key: String,
        #[serde(flatten)]
        expect: ValueMatcher,
        /// When true, a missing key is a violation. When false, a missing key
        /// is ignored and only a present-but-wrong value is a violation.
        #[serde(default = "crate::schema::default_true")]
        required: bool,
    },
    /// A key path resolves to a list, and one value equals `value`.
    ///
    /// `manifest_key_value` tests every value and reports each one that
    /// differs, which suits "no permission may be X". This check reports once
    /// when nothing matches, which suits "the app must declare X".
    ///
    /// A live compile run showed why the pair is needed: the model tried to
    /// write a value filter inside a key path, and no such syntax exists.
    ManifestKeyContains {
        file: ManifestFile,
        key: String,
        value: serde_json::Value,
    },
    /// The entitlement key exists.
    EntitlementPresent { key: String },
    /// The entitlement key does not exist.
    EntitlementAbsent { key: String },
    /// A regular expression matches source file contents.
    RegexSource {
        pattern: String,
        /// Reverse the reading of the check.
        ///
        /// With the default `false` the check reports every match, which suits
        /// a banned API. With `true` it reports once when nothing matches,
        /// which suits a required call. A rule that says "an app with this
        /// entitlement must call this API" needs the second reading.
        #[serde(default)]
        expect_present: bool,
        #[serde(default = "default_source_include")]
        include: Vec<String>,
        #[serde(default)]
        exclude: Vec<String>,
        #[serde(default)]
        case_insensitive: bool,
        /// Stop after this many matches. `None` means report every match.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_matches: Option<usize>,
    },
    /// A specific API symbol appears in source.
    ///
    /// `require_any_import` cuts false positives: when the list is not empty,
    /// the file must also contain one of the listed import statements.
    ApiUsage {
        symbol: String,
        /// Reverse the reading of the check. See `regex_source`.
        #[serde(default)]
        expect_present: bool,
        #[serde(default)]
        require_any_import: Vec<String>,
        #[serde(default = "default_source_include")]
        include: Vec<String>,
        #[serde(default)]
        exclude: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_matches: Option<usize>,
    },
    /// A direct dependency carries a flag in the knowledge base.
    DependencyFlag {
        flag_type: DependencyFlagType,
        #[serde(default)]
        ecosystems: Vec<Ecosystem>,
    },
    /// A requirement inside the Apple privacy manifest.
    PrivacyManifest {
        #[serde(flatten)]
        requirement: PrivacyManifestRequirement,
    },
    /// A build setting satisfies the matcher.
    ConfigValue {
        file: ConfigFile,
        setting: String,
        #[serde(flatten)]
        expect: ValueMatcher,
        #[serde(default = "crate::schema::default_true")]
        required: bool,
    },
}

pub(crate) fn default_true() -> bool {
    true
}

impl MechanicalCheck {
    /// The check type token, matching the serialized `check_type` field.
    pub fn check_type(&self) -> &'static str {
        match self {
            MechanicalCheck::ManifestKeyPresent { .. } => "manifest_key_present",
            MechanicalCheck::ManifestKeyAbsent { .. } => "manifest_key_absent",
            MechanicalCheck::ManifestKeyValue { .. } => "manifest_key_value",
            MechanicalCheck::ManifestKeyContains { .. } => "manifest_key_contains",
            MechanicalCheck::EntitlementPresent { .. } => "entitlement_present",
            MechanicalCheck::EntitlementAbsent { .. } => "entitlement_absent",
            MechanicalCheck::RegexSource { .. } => "regex_source",
            MechanicalCheck::ApiUsage { .. } => "api_usage",
            MechanicalCheck::DependencyFlag { .. } => "dependency_flag",
            MechanicalCheck::PrivacyManifest { .. } => "privacy_manifest",
            MechanicalCheck::ConfigValue { .. } => "config_value",
        }
    }

    /// The target class this check reads.
    pub fn target(&self) -> Target {
        match self {
            MechanicalCheck::ManifestKeyPresent { .. }
            | MechanicalCheck::ManifestKeyAbsent { .. }
            | MechanicalCheck::ManifestKeyValue { .. }
            | MechanicalCheck::ManifestKeyContains { .. }
            | MechanicalCheck::PrivacyManifest { .. } => Target::Manifest,
            MechanicalCheck::EntitlementPresent { .. }
            | MechanicalCheck::EntitlementAbsent { .. } => Target::Entitlements,
            MechanicalCheck::RegexSource { .. } | MechanicalCheck::ApiUsage { .. } => {
                Target::Source
            }
            MechanicalCheck::DependencyFlag { .. } => Target::Dependency,
            MechanicalCheck::ConfigValue { .. } => Target::Config,
        }
    }
}

/// Where a rule comes from in the canonical policy documents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    /// The source document identifier, for example `apple-app-store-review-guidelines`.
    pub document: String,
    /// The section or paragraph reference, for example `5.1.1`.
    pub section: String,
    /// The date or version of the source document.
    pub document_version: String,
    /// A deep link to the specific section.
    pub url: String,
}

/// One plausible reading of an interpretive rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interpretation {
    pub interpretation: String,
    pub likelihood: Likelihood,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

/// What a developer must do to comply.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Remediation {
    pub summary: String,
    #[serde(default)]
    pub examples: Vec<String>,
    /// Anonymized real-world approaches that worked, drawn from forum evidence.
    #[serde(default)]
    pub forum_insights: Vec<String>,
}

/// A condition that must hold before a rule is evaluated at all.
///
/// Most policy is conditional. A Kids Category rule applies to a kids app. A
/// VPN rule applies to an app that ships a VPN. A GDPR rule applies to an app
/// that reaches the European Union.
///
/// Without a precondition the checker has two bad choices: run the rule on
/// every project and report a false violation, or drop the rule. A live
/// compile run produced six rules that hit exactly this, and each one fired on
/// every app in the validation corpus.
///
/// A precondition that cannot be decided leaves the rule unevaluated. An
/// unknown context never turns into a finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "when", rename_all = "snake_case")]
pub enum Precondition {
    /// A mechanical check that finds nothing.
    ///
    /// The check keeps its usual meaning. `entitlement_present` reports a
    /// finding when the entitlement is absent, so as a precondition it holds
    /// when the entitlement is present.
    CheckPasses { check: Box<MechanicalCheck> },
    /// A mechanical check that finds something.
    ///
    /// Use it for the opposite reading, for example a rule that applies only
    /// when a permission is declared.
    CheckFails { check: Box<MechanicalCheck> },
    /// One of the conditions holds.
    ///
    /// Every other precondition on a rule is joined with and. A policy that
    /// names alternatives needs or, and without it a rule has two bad choices.
    /// Guideline 4.5.4 covers an app that blocks calls, SMS, or MMS, and an
    /// app does that through a `CallKit` entitlement, an SMS filter extension,
    /// or its own logic. A rule that named only the entitlement missed the
    /// other two, and a rule that named nothing applied to every app. Two
    /// verification rounds graded the same rule understated and then
    /// overreach, one for each of those choices.
    AnyOf { conditions: Vec<Precondition> },
    /// A fact in `.pekorc.json` equals a value.
    FactEquals {
        key: String,
        value: serde_json::Value,
    },
    /// A fact in `.pekorc.json` is a list that holds a value.
    ///
    /// Use it for the places an app ships, for example
    /// `{"when": "fact_contains", "key": "distributes_in", "value": "eu"}`.
    FactContains {
        key: String,
        value: serde_json::Value,
    },
    /// A fact is declared at all, whatever its value.
    FactPresent { key: String },
}

impl Precondition {
    /// The fact this precondition reads, when it reads one.
    pub fn fact_key(&self) -> Option<&str> {
        match self {
            Precondition::FactEquals { key, .. }
            | Precondition::FactContains { key, .. }
            | Precondition::FactPresent { key } => Some(key),
            _ => None,
        }
    }

    /// The check this precondition runs, when it runs one.
    pub fn check(&self) -> Option<&MechanicalCheck> {
        match self {
            Precondition::CheckPasses { check } | Precondition::CheckFails { check } => Some(check),
            _ => None,
        }
    }

    /// This condition, and every condition nested inside it.
    ///
    /// `any_of` holds other conditions, and a validator that reads only the
    /// top level would let a broken check inside one through.
    pub fn flatten(&self) -> Vec<&Precondition> {
        let mut out = vec![self];
        if let Precondition::AnyOf { conditions } = self {
            for inner in conditions {
                out.extend(inner.flatten());
            }
        }
        out
    }
}

/// How the checker detects a violation of a rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detection {
    /// The project data classes this rule reads.
    #[serde(default)]
    pub targets: Vec<Target>,
    /// Conditions that must all hold before the rule is evaluated.
    ///
    /// An empty list means the rule applies to every project.
    #[serde(default)]
    pub applies_when: Vec<Precondition>,
    /// The deterministic checks. Empty for a purely interpretive rule.
    #[serde(default)]
    pub mechanical_checks: Vec<MechanicalCheck>,
    /// The evaluation prompt for an interpretive rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpretive_prompt: Option<String>,
    /// Background context supplied to the model with the prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpretive_context: Option<String>,
    /// Glob patterns that select the files sent to the model. The scoping pass
    /// uses these so that the model never receives the whole codebase.
    #[serde(default)]
    pub interpretive_scope: Vec<String>,
}

/// One entry in the rule change history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangelogEntry {
    pub date: DateTime<Utc>,
    pub change: String,
}

/// A single compliance rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub rule_id: RuleId,
    /// The revision of this rule. It increases on every content change.
    pub version: u32,
    pub platform: Platform,
    pub source: SourceRef,
    pub rule_type: RuleType,
    pub category: Category,
    pub severity: Severity,
    /// A short human readable title.
    pub title: String,
    /// The full description of the rule.
    pub description: String,
    pub detection: Detection,
    #[serde(default)]
    pub interpretations: Vec<Interpretation>,
    pub remediation: Remediation,
    /// True if a user can acknowledge a finding and exclude it from pass/fail.
    pub overridable: bool,
    /// The human validation state. Only `validated` rules run.
    #[serde(default)]
    pub status: RuleStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub changelog: Vec<ChangelogEntry>,
}

impl Rule {
    pub fn is_mechanical(&self) -> bool {
        self.rule_type == RuleType::Mechanical
    }

    pub fn is_interpretive(&self) -> bool {
        self.rule_type == RuleType::Interpretive
    }

    /// True if this rule runs against a submission for `target`.
    pub fn applies_to_platform(&self, target: Platform) -> bool {
        self.platform.applies_to(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_orders_low_to_high() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
        assert_eq!(Severity::Error.downgrade(), Severity::Warning);
        assert_eq!(Severity::Info.downgrade(), Severity::Info);
    }

    #[test]
    fn mechanical_check_uses_check_type_and_parameters() {
        let check = MechanicalCheck::ManifestKeyPresent {
            file: ManifestFile::InfoPlist,
            key: "NSCameraUsageDescription".into(),
        };
        let json = serde_json::to_value(&check).unwrap();
        assert_eq!(json["check_type"], "manifest_key_present");
        assert_eq!(json["parameters"]["file"], "info_plist");
        assert_eq!(json["parameters"]["key"], "NSCameraUsageDescription");
        let back: MechanicalCheck = serde_json::from_value(json).unwrap();
        assert_eq!(back, check);
    }

    #[test]
    fn value_matcher_flattens_into_parameters() {
        let check = MechanicalCheck::ConfigValue {
            file: ConfigFile::BuildGradle,
            setting: "targetSdkVersion".into(),
            expect: ValueMatcher::MinInt { value: 35 },
            required: true,
        };
        let json = serde_json::to_value(&check).unwrap();
        assert_eq!(json["parameters"]["match"], "min_int");
        assert_eq!(json["parameters"]["value"], 35);
        let back: MechanicalCheck = serde_json::from_value(json).unwrap();
        assert_eq!(back, check);
    }

    #[test]
    fn a_source_check_can_require_a_call_instead_of_banning_one() {
        let json = serde_json::json!({
            "check_type": "api_usage",
            "parameters": {
                "symbol": "NEVPNManager",
                "expect_present": true,
                "include": ["**/*.swift"]
            }
        });
        let check: MechanicalCheck = serde_json::from_value(json).unwrap();
        let MechanicalCheck::ApiUsage { expect_present, .. } = check else {
            panic!("expected an api usage check");
        };
        assert!(expect_present, "the check must read as a requirement");
    }

    #[test]
    fn a_precondition_round_trips() {
        let json = serde_json::json!({
            "when": "fact_contains",
            "key": "distributes_in",
            "value": "eu"
        });
        let condition: Precondition = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(condition.fact_key(), Some("distributes_in"));
        assert_eq!(serde_json::to_value(&condition).unwrap(), json);
    }

    #[test]
    fn a_rule_applies_everywhere_when_it_names_no_precondition() {
        let detection: Detection = serde_json::from_value(serde_json::json!({
            "targets": ["source"],
            "mechanical_checks": []
        }))
        .unwrap();
        assert!(detection.applies_when.is_empty());
    }

    #[test]
    fn check_type_token_matches_serialized_tag() {
        let checks = [
            MechanicalCheck::EntitlementAbsent { key: "k".into() },
            MechanicalCheck::RegexSource {
                pattern: "x".into(),
                expect_present: false,
                include: vec![],
                exclude: vec![],
                case_insensitive: false,
                max_matches: None,
            },
            MechanicalCheck::PrivacyManifest {
                requirement: PrivacyManifestRequirement::FilePresent,
            },
        ];
        for check in checks {
            let json = serde_json::to_value(&check).unwrap();
            assert_eq!(json["check_type"], check.check_type());
        }
    }
}
