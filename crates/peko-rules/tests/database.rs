//! The shipped rule database must load and validate.

use peko_rules::{Platform, RuleDatabase, RuleStatus, RuleType};
use std::path::PathBuf;

fn rules_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rules")
}

fn database() -> RuleDatabase {
    RuleDatabase::load_from_dir(rules_dir()).expect("the rule database must load")
}

#[test]
fn the_database_loads() {
    let db = database();
    assert!(!db.is_empty(), "the database must hold rules");
    assert_eq!(db.manifest().schema_version, 1);
}

#[test]
fn every_rule_is_valid() {
    let issues = database().validate();
    assert!(
        issues.is_empty(),
        "the database holds {} validation issues:\n{}",
        issues.len(),
        issues
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn both_platforms_have_active_rules() {
    let db = database();
    assert!(!db.active_rules(Platform::Ios).is_empty());
    assert!(!db.active_rules(Platform::Android).is_empty());
}

#[test]
fn cross_platform_rules_reach_both_platforms() {
    let db = database();
    let ios: Vec<_> = db
        .active_rules(Platform::Ios)
        .into_iter()
        .filter(|rule| rule.platform == Platform::Both)
        .collect();
    let android: Vec<_> = db
        .active_rules(Platform::Android)
        .into_iter()
        .filter(|rule| rule.platform == Platform::Both)
        .collect();
    assert_eq!(ios.len(), android.len());
    assert!(
        !ios.is_empty(),
        "the seed database must hold a cross-platform rule"
    );
}

#[test]
fn the_database_holds_both_rule_types() {
    let db = database();
    let mechanical = db
        .rules()
        .iter()
        .filter(|rule| rule.is_mechanical())
        .count();
    let interpretive = db
        .rules()
        .iter()
        .filter(|rule| rule.is_interpretive())
        .count();
    assert!(mechanical > 0, "the database must hold mechanical rules");
    // This repository ships mechanical rules only. The engine here skips
    // every other kind, so an interpretive rule that reached this database
    // would run nowhere and would carry its interpretations and its forum
    // evidence into a public tree for no benefit.
    assert_eq!(
        interpretive, 0,
        "an interpretive rule reached the open database"
    );
    assert_eq!(mechanical + interpretive, db.len());
}

#[test]
fn every_rule_passed_the_human_validation_gate() {
    let db = database();
    let candidates: Vec<String> = db
        .rules()
        .iter()
        .filter(|rule| rule.status != RuleStatus::Validated)
        .map(|rule| rule.rule_id.to_string())
        .collect();
    assert!(
        candidates.is_empty(),
        "these rules are not validated and would not run: {candidates:?}"
    );
}

#[test]
fn interpretive_rules_limit_the_files_sent_to_the_model() {
    for rule in database().rules() {
        if rule.rule_type == RuleType::Interpretive {
            assert!(
                !rule.detection.interpretive_scope.is_empty(),
                "{} sends the whole codebase to the model",
                rule.rule_id
            );
        }
    }
}
