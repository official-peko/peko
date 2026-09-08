//! What a project must answer before the rules can speak.
//!
//! A `.pekorc.json` that answers every fact in the vocabulary is long, and
//! most of it is dead weight: a rule about kids advertising asks nothing of a
//! banking app. A file that answers nothing is worse, because every rule that
//! needs an answer reports undecided, and undecided reads like a pass.
//!
//! This module takes the middle. It runs every rule against the project, and
//! it keeps the facts that actually stop a rule from deciding. Those are the
//! questions. The rest need no answer, because no rule for this project reads
//! them.

use std::collections::BTreeMap;

use peko_rules::{facts, RuleDatabase};
use serde::Serialize;

use crate::config::PekoConfig;
use crate::engine::{rule_applies, Applicability};
use crate::project::Project;

/// Where an answer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A person wrote it in `.pekorc.json`.
    Declared,
    /// The checker read it out of the project.
    Inferred,
    /// Nobody answered it, and the vocabulary carries a safe default.
    ///
    /// A default is not evidence. It is the answer that holds for most apps,
    /// and it is always the answer that keeps a rule quiet, so a project that
    /// differs must say so.
    Default,
}

/// A fact this project still owes an answer for.
#[derive(Debug, Clone, Serialize)]
pub struct Question {
    pub fact: String,
    /// The question to put to a person.
    pub prompt: String,
    /// The shape the answer takes, for example `boolean` or `string[]`.
    pub shape: String,
    /// The rules that wait on it, most important first.
    pub blocks: Vec<String>,
}

/// A value written into a list fact that no rule anywhere reads.
///
/// Almost always a spelling. `distributes_in` takes `eu`, `US`, and `US-CA`,
/// and a file that says `GB` reads as answered while switching nothing on.
/// Nothing complained, and the rules those 33 entries gate stayed quiet.
#[derive(Debug, Clone, Serialize)]
pub struct Unread {
    pub fact: String,
    pub value: String,
    /// The values that do switch a rule on, in the order the database holds.
    pub understood: Vec<String>,
}

/// An answer already in hand.
#[derive(Debug, Clone, Serialize)]
pub struct Answer {
    pub fact: String,
    pub value: serde_json::Value,
    pub source: Source,
}

/// The gap between what the project says and what the rules need.
#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub answered: Vec<Answer>,
    pub questions: Vec<Question>,
    /// Values written into a list fact that no rule reads.
    pub unread: Vec<Unread>,
    /// Rules that decide without any further answer.
    pub decided: usize,
}

impl Plan {
    /// True when every rule that applies to this project can decide.
    pub fn complete(&self) -> bool {
        self.questions.is_empty()
    }

    /// The `facts` block to write into `.pekorc.json`.
    ///
    /// An inferred answer is written out the same as a declared one. Writing
    /// it makes the guess visible and editable, and a person who disagrees
    /// changes the line. An unanswered fact is written as `null`, so the file
    /// carries its own to-do list.
    pub fn config_block(&self) -> serde_json::Value {
        let mut block = serde_json::Map::new();
        for answer in &self.answered {
            block.insert(answer.fact.clone(), answer.value.clone());
        }
        for question in &self.questions {
            block.insert(question.fact.clone(), serde_json::Value::Null);
        }
        serde_json::Value::Object(block)
    }
}

fn shape_name(shape: facts::Shape) -> &'static str {
    match shape {
        facts::Shape::Bool => "boolean",
        facts::Shape::Text => "string",
        facts::Shape::Integer => "integer",
        facts::Shape::TextList => "string[]",
    }
}

/// Work out what this project must still answer.
pub fn plan(project: &Project, config: &PekoConfig, database: &RuleDatabase) -> Plan {
    let mut blocking: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut decided = 0usize;

    for rule in database.rules() {
        if !rule.applies_to_platform(project.platform) {
            continue;
        }
        match rule_applies(rule, project, config) {
            Applicability::Undecided(reason) => match facts::canonical(&reason) {
                Some(name) => blocking
                    .entry(name.to_string())
                    .or_default()
                    .push(rule.rule_id.to_string()),
                // A check with no input to read is not a question for a
                // person. Nobody can answer it by typing, so it is not here.
                None => decided += 1,
            },
            _ => decided += 1,
        }
    }

    let mut answered = Vec::new();
    for fact in facts::declared() {
        if let Some(value) = config.fact(fact.name) {
            answered.push(Answer {
                fact: fact.name.to_string(),
                value: value.clone(),
                source: Source::Declared,
            });
        } else if let Some(value) = project.derived_facts.get(fact.name) {
            answered.push(Answer {
                fact: fact.name.to_string(),
                value: value.clone(),
                source: if project.inferred_facts.iter().any(|name| name == fact.name) {
                    Source::Inferred
                } else if project.assumed_facts.iter().any(|name| name == fact.name) {
                    Source::Default
                } else {
                    Source::Inferred
                },
            });
        }
    }

    let questions = blocking
        .into_iter()
        .map(|(name, mut blocks)| {
            blocks.sort();
            blocks.dedup();
            let entry = facts::lookup(&name);
            Question {
                prompt: entry.map_or_else(
                    || format!("What is {name}?"),
                    |fact| fact.question.to_string(),
                ),
                shape: entry
                    .map_or("string", |fact| shape_name(fact.shape))
                    .to_string(),
                fact: name,
                blocks,
            }
        })
        .collect();

    Plan {
        unread: unread_values(config, database),
        answered,
        questions,
        decided,
    }
}

/// Values in a list fact that match no precondition in the database.
///
/// A list fact is free text, so a wrong entry is silent: it reads as an
/// answer, and every rule it was meant to switch on stays off. Only
/// `fact_contains` is considered, because that is the only test that compares
/// against a value inside a list.
fn unread_values(config: &PekoConfig, database: &RuleDatabase) -> Vec<Unread> {
    let mut understood: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for rule in database.rules() {
        for condition in &rule.detection.applies_when {
            collect_contains(condition, &mut understood);
        }
    }

    let mut unread = Vec::new();
    for fact in facts::declared().filter(|fact| fact.shape == facts::Shape::TextList) {
        let Some(known) = understood.get(fact.name) else {
            // No rule reads this fact by value at all, so nothing about the
            // entries can be wrong.
            continue;
        };
        let Some(written) = config.fact(fact.name).and_then(|v| v.as_array()) else {
            continue;
        };
        for value in written.iter().filter_map(|v| v.as_str()) {
            if !known.iter().any(|k| k == value) {
                unread.push(Unread {
                    fact: fact.name.to_string(),
                    value: value.to_string(),
                    understood: known.clone(),
                });
            }
        }
    }
    unread
}

/// Walk a precondition, including the ones that nest others.
fn collect_contains<'a>(
    condition: &'a peko_rules::Precondition,
    into: &mut BTreeMap<&'a str, Vec<String>>,
) {
    match condition {
        peko_rules::Precondition::FactContains { key, value } => {
            if let Some(text) = value.as_str() {
                let known = into.entry(key.as_str()).or_default();
                if !known.iter().any(|k| k == text) {
                    known.push(text.to_string());
                }
            }
        }
        peko_rules::Precondition::AnyOf { conditions } => {
            for nested in conditions {
                collect_contains(nested, into);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod unread_tests {
    use super::unread_values;
    use crate::config::PekoConfig;

    fn database() -> peko_rules::RuleDatabase {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("the repository root")
            .join("rules");
        peko_rules::RuleDatabase::load_from_dir(root).expect("the rule database loads")
    }

    fn config(places: &[&str]) -> PekoConfig {
        let json = serde_json::json!({
            "version": 1,
            "platform": "ios",
            "facts": {"distributes_in": places},
        });
        serde_json::from_value(json).expect("the config parses")
    }

    /// The bug this catches. `distributes_in` takes `eu`, `US`, and `US-CA`,
    /// and two examples shipped saying `GB` and `DE`. Both parsed, both read
    /// as answered, and every rule those entries were meant to switch on
    /// stayed off. Nothing said so.
    #[test]
    fn a_place_no_rule_reads_is_named() {
        let found = unread_values(&config(&["US", "GB", "DE"]), &database());
        let values: Vec<&str> = found.iter().map(|u| u.value.as_str()).collect();
        assert_eq!(values, ["GB", "DE"], "{found:?}");
        assert!(
            found[0].understood.iter().any(|k| k == "eu"),
            "the message has to say what does work: {:?}",
            found[0].understood
        );
    }

    #[test]
    fn the_values_the_rules_read_are_not_named() {
        assert!(unread_values(&config(&["US", "US-CA", "eu"]), &database()).is_empty());
    }

    /// A fact nobody wrote cannot hold a wrong value.
    #[test]
    fn an_unanswered_fact_says_nothing() {
        let json = serde_json::json!({"version": 1, "platform": "ios", "facts": {}});
        let config: PekoConfig = serde_json::from_value(json).expect("the config parses");
        assert!(unread_values(&config, &database()).is_empty());
    }
}
