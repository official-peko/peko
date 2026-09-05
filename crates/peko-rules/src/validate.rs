//! Rule invariant checks.
//!
//! Validation collects every problem before it returns. A rule author sees the
//! complete list in one pass.

use crate::error::ValidationIssue;
use crate::platform::Platform;
use crate::schema::{
    ManifestFile, MechanicalCheck, Precondition, PrivacyManifestRequirement, Rule, RuleType,
    ValueMatcher,
};

/// Check one rule and return every problem found.
///
/// The function reads as one long list of checks on purpose. Splitting it into
/// helpers hides which invariants a rule must satisfy.
#[allow(clippy::too_many_lines)]
pub fn validate_rule(rule: &Rule) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let id = rule.rule_id.to_string();
    let mut push = |field: &str, message: String| {
        issues.push(ValidationIssue {
            rule_id: id.clone(),
            field: field.to_string(),
            message,
        });
    };

    if rule.rule_id.platform() != rule.platform {
        push(
            "platform",
            format!(
                "rule id prefix {} does not match platform {}",
                rule.rule_id.platform().prefix(),
                rule.platform
            ),
        );
    }
    if rule.rule_id.category() != rule.category {
        push(
            "category",
            format!(
                "rule id category {} does not match category {}",
                rule.rule_id.category(),
                rule.category
            ),
        );
    }
    if rule.version == 0 {
        push("version", "version must start at 1".into());
    }
    if rule.title.trim().is_empty() {
        push("title", "title must not be empty".into());
    }
    if rule.description.trim().is_empty() {
        push("description", "description must not be empty".into());
    }
    if rule.remediation.summary.trim().is_empty() {
        push("remediation.summary", "summary must not be empty".into());
    }
    if rule.source.document.trim().is_empty() {
        push("source.document", "document must not be empty".into());
    }
    if rule.source.section.trim().is_empty() {
        push("source.section", "section must not be empty".into());
    }
    if !rule.source.url.starts_with("https://") {
        push("source.url", "url must be an https link".into());
    }
    if rule.updated_at < rule.created_at {
        push("updated_at", "updated_at is before created_at".into());
    }

    match rule.rule_type {
        RuleType::Mechanical => {
            if rule.detection.mechanical_checks.is_empty() {
                push(
                    "detection.mechanical_checks",
                    "a mechanical rule must define at least one check".into(),
                );
            }
            if rule.detection.interpretive_prompt.is_some() {
                push(
                    "detection.interpretive_prompt",
                    "a mechanical rule must not carry an interpretive prompt".into(),
                );
            }
        }
        RuleType::Interpretive => {
            match &rule.detection.interpretive_prompt {
                None => push(
                    "detection.interpretive_prompt",
                    "an interpretive rule must define an evaluation prompt".into(),
                ),
                Some(prompt) if prompt.trim().len() < 20 => push(
                    "detection.interpretive_prompt",
                    "the evaluation prompt is too short to be useful".into(),
                ),
                Some(_) => {}
            }
            if rule.interpretations.is_empty() {
                push(
                    "interpretations",
                    "an interpretive rule must list at least one interpretation".into(),
                );
            }
            if !rule.overridable {
                push(
                    "overridable",
                    "an interpretive rule must be overridable".into(),
                );
            }
            if rule.detection.interpretive_scope.is_empty() {
                push(
                    "detection.interpretive_scope",
                    "an interpretive rule must limit the files sent to the model".into(),
                );
            }
        }
    }

    if rule.detection.targets.is_empty() {
        push("detection.targets", "targets must not be empty".into());
    }
    for check in &rule.detection.mechanical_checks {
        let target = check.target();
        if !rule.detection.targets.contains(&target) {
            push(
                "detection.targets",
                format!(
                    "check {} reads target {target:?} which is not declared",
                    check.check_type()
                ),
            );
        }
    }

    for (index, check) in rule.detection.mechanical_checks.iter().enumerate() {
        let field = format!("detection.mechanical_checks[{index}]");
        validate_check(rule, check, &field, &mut issues);
    }

    for (index, outer) in rule.detection.applies_when.iter().enumerate() {
        // `any_of` nests conditions, so every level is checked, not the top.
        for condition in outer.flatten() {
            let field = format!("detection.applies_when[{index}]");
            if let Some(check) = condition.check() {
                validate_check(rule, check, &field, &mut issues);
            }
            if let Precondition::AnyOf { conditions } = condition {
                if conditions.is_empty() {
                    issues.push(ValidationIssue {
                        rule_id: rule.rule_id.to_string(),
                        field: field.clone(),
                        message: "any_of holds no condition, so it never holds".to_string(),
                    });
                }
            }
            if let Some(key) = condition.fact_key() {
                if key.trim().is_empty() {
                    issues.push(ValidationIssue {
                        rule_id: rule.rule_id.to_string(),
                        field: field.clone(),
                        message: "a fact key must not be empty".to_string(),
                    });
                } else if crate::facts::canonical(key).is_none() {
                    // Nothing used to check a fact name, so the compiler wrote
                    // a new one for almost every rule. A rule naming a fact
                    // that no developer will ever declare never fires.
                    issues.push(ValidationIssue {
                        rule_id: rule.rule_id.to_string(),
                        field: field.clone(),
                        message: format!(
                            "the fact `{key}` is not in the vocabulary. \
                             Add it to peko-rules/src/facts.rs, or use a name that is"
                        ),
                    });
                } else if crate::facts::is_alias(key) {
                    let canonical = crate::facts::canonical(key).unwrap_or(key);
                    issues.push(ValidationIssue {
                        rule_id: rule.rule_id.to_string(),
                        field: field.clone(),
                        message: format!("the fact `{key}` is an old name for `{canonical}`"),
                    });
                }
            }
            // A precondition that repeats a rule check makes the rule unable to
            // fire: the condition holds only when the check finds nothing, and
            // then the same check finds nothing again.
            if let (Precondition::CheckPasses { check }, true) = (
                condition,
                rule.detection
                    .mechanical_checks
                    .iter()
                    .any(|other| Some(other) == condition.check()),
            ) {
                let _ = check;
                issues.push(ValidationIssue {
                    rule_id: rule.rule_id.to_string(),
                    field,
                    message: "a precondition repeats a rule check, so the rule can never fire"
                        .to_string(),
                });
            }
        }
    }

    issues
}

#[allow(clippy::too_many_lines)]
fn validate_check(
    rule: &Rule,
    check: &MechanicalCheck,
    field: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let id = rule.rule_id.to_string();
    let mut push = |message: String| {
        issues.push(ValidationIssue {
            rule_id: id.clone(),
            field: field.to_string(),
            message,
        });
    };

    let check_regex = |pattern: &str, push: &mut dyn FnMut(String)| {
        if let Err(err) = regex::Regex::new(pattern) {
            push(format!("invalid regular expression {pattern:?}: {err}"));
        }
    };

    match check {
        MechanicalCheck::BundleEntry { pattern, .. } => {
            if pattern.trim().is_empty() {
                push("pattern must not be empty".into());
            }
            // A pattern with no slash matches an entry at the root of the
            // archive and nothing else, which is almost never what a rule
            // means. Every path in an ipa starts Payload/ and in an aab
            // base/.
            if !pattern.contains('/') {
                push(format!(
                    "pattern {pattern} names no directory, so it matches only the root of the archive"
                ));
            }
        }
        MechanicalCheck::ManifestKeyPresent { file, key }
        | MechanicalCheck::ManifestKeyContains { file, key, .. }
        | MechanicalCheck::ManifestKeyAbsent { file, key } => {
            if key.trim().is_empty() {
                push("key must not be empty".into());
            }
            check_manifest_platform(rule.platform, *file, &mut push);
        }
        MechanicalCheck::ManifestKeyValue {
            file, key, expect, ..
        } => {
            if key.trim().is_empty() {
                push("key must not be empty".into());
            }
            check_manifest_platform(rule.platform, *file, &mut push);
            validate_matcher(expect, &mut push);
        }
        MechanicalCheck::EntitlementPresent { key }
        | MechanicalCheck::EntitlementAbsent { key } => {
            if key.trim().is_empty() {
                push("key must not be empty".into());
            }
            if rule.platform == Platform::Android {
                push("entitlement checks apply to iOS only".into());
            }
        }
        MechanicalCheck::RegexSource {
            pattern, include, ..
        } => {
            check_regex(pattern, &mut push);
            if include.is_empty() {
                push("include must list at least one glob".into());
            }
        }
        MechanicalCheck::ApiUsage {
            symbol, include, ..
        } => {
            check_regex(symbol, &mut push);
            if include.is_empty() {
                push("include must list at least one glob".into());
            }
        }
        MechanicalCheck::DependencyFlag { ecosystems, .. } => {
            if ecosystems.is_empty() {
                push("ecosystems must list at least one ecosystem".into());
            }
        }
        MechanicalCheck::PrivacyManifest { requirement } => {
            if rule.platform == Platform::Android {
                push("privacy manifest checks apply to iOS only".into());
            }
            if let PrivacyManifestRequirement::ApiReasonDeclared {
                api_type,
                allowed_reasons,
                triggered_by,
            } = requirement
            {
                if !api_type.starts_with("NSPrivacyAccessedAPICategory") {
                    push(format!(
                        "api_type {api_type:?} must start with NSPrivacyAccessedAPICategory"
                    ));
                }
                if allowed_reasons.is_empty() {
                    push("allowed_reasons must list the accepted reason codes".into());
                }
                if triggered_by.is_empty() {
                    push(
                        "triggered_by must list the source symbols that require the declaration"
                            .into(),
                    );
                }
            }
        }
        MechanicalCheck::ConfigValue {
            file,
            setting,
            expect,
            ..
        } => {
            if setting.trim().is_empty() {
                push("setting must not be empty".into());
            }
            match (rule.platform, file) {
                (Platform::Android, crate::schema::ConfigFile::XcodeProject) => {
                    push("xcode project checks apply to iOS only".into());
                }
                (Platform::Ios, crate::schema::ConfigFile::BuildGradle) => {
                    push("build.gradle checks apply to Android only".into());
                }
                _ => {}
            }
            validate_matcher(expect, &mut push);
        }
    }
}

fn check_manifest_platform(platform: Platform, file: ManifestFile, push: &mut dyn FnMut(String)) {
    let required = match file {
        ManifestFile::InfoPlist | ManifestFile::PrivacyManifest | ManifestFile::Entitlements => {
            Platform::Ios
        }
        ManifestFile::AndroidManifest => Platform::Android,
    };
    if platform != Platform::Both && platform != required {
        push(format!(
            "file {file:?} belongs to {required} but the rule targets {platform}"
        ));
    }
}

fn validate_matcher(matcher: &ValueMatcher, push: &mut dyn FnMut(String)) {
    match matcher {
        ValueMatcher::Regex { pattern } | ValueMatcher::NotRegex { pattern } => {
            if let Err(err) = regex::Regex::new(pattern) {
                push(format!("invalid regular expression {pattern:?}: {err}"));
            }
        }
        ValueMatcher::OneOf { values } if values.is_empty() => {
            push("one_of must list at least one value".into());
        }
        _ => {}
    }
}
