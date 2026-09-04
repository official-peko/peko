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
        answered,
        questions,
        decided,
    }
}
