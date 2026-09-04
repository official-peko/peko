//! The questionnaire, and the inference that shortens it.
//!
//! Two properties matter more than the rest, and both are tested here.
//!
//! The first is direction. Inference answers a fact `true` when it sees
//! evidence, and it leaves the fact unanswered when it sees none. It never
//! answers `false`. A wrong `true` makes a rule run that did not need to, and
//! the corpus finds that within a day. A wrong `false` makes a rule stay
//! silent, the report looks clean, and nothing finds it at all.
//!
//! The second is precedence. A person who writes an answer in `.pekorc.json`
//! overrules the guess, in both directions.

use peko_check::plan::{self, Source};
use peko_check::{PekoConfig, Project};
use peko_rules::{Platform, RuleDatabase};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn database() -> RuleDatabase {
    RuleDatabase::load_from_dir(root().join("rules")).expect("the rule database must load")
}

/// Build a project from a directory of files written for one test.
fn scratch(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("peko-plan-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, body) in files {
        let target = dir.join(path);
        std::fs::create_dir_all(target.parent().expect("a parent")).expect("create the directory");
        std::fs::write(&target, body).expect("write the file");
    }
    dir
}

fn load(dir: &Path, config: &PekoConfig) -> Project {
    Project::load(dir, config).expect("the project must load")
}

const PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.example.app</string>
</dict></plist>
"#;

#[test]
fn a_sign_up_screen_answers_that_the_app_has_accounts() {
    let dir = scratch(
        "accounts",
        &[
            ("Info.plist", PLIST),
            (
                "Sources/Login.swift",
                "func signUp(email: String) { createAccount(email) }\n",
            ),
        ],
    );
    let config = PekoConfig::new(Platform::Ios);
    let project = load(&dir, &config);
    assert_eq!(
        project.derived_facts.get("has_user_accounts"),
        Some(&serde_json::json!(true)),
        "a file that calls signUp says the app has accounts"
    );
    assert!(
        project
            .inferred_facts
            .iter()
            .any(|name| name == "has_user_accounts"),
        "an inference must say it is an inference, so the report can print it"
    );
}

#[test]
fn accounts_imply_that_the_app_collects_personal_data() {
    let dir = scratch(
        "accounts-imply",
        &[
            ("Info.plist", PLIST),
            ("Sources/Login.swift", "func signUp() {}\n"),
        ],
    );
    let config = PekoConfig::new(Platform::Ios);
    let project = load(&dir, &config);
    assert_eq!(
        project.derived_facts.get("collects_personal_data"),
        Some(&serde_json::json!(true)),
        "an app that signs people in holds data about them"
    );
}

#[test]
fn no_evidence_leaves_the_fact_unanswered_rather_than_false() {
    // This is the property the whole design rests on. A project with no sign
    // up screen may still have accounts, drawn from a server or written in a
    // file this scan did not read. Answering `false` here would silence every
    // account rule and nothing downstream would notice.
    let dir = scratch(
        "quiet",
        &[
            ("Info.plist", PLIST),
            ("Sources/Empty.swift", "struct Empty {}\n"),
        ],
    );
    let config = PekoConfig::new(Platform::Ios);
    let project = load(&dir, &config);
    for fact in [
        "has_user_accounts",
        "collects_personal_data",
        "has_user_generated_content",
        "collects_birthdate",
        "shows_consent_ui",
        "sells_or_shares_personal_information",
        "uses_third_party_processor",
    ] {
        assert_eq!(
            project.derived_facts.get(fact),
            None,
            "{fact} must stay unanswered without evidence, never false"
        );
    }
}

#[test]
fn a_declared_answer_beats_an_inference_in_both_directions() {
    let dir = scratch(
        "declared",
        &[
            ("Info.plist", PLIST),
            ("Sources/Login.swift", "func signUp() {}\n"),
        ],
    );
    let mut config = PekoConfig::new(Platform::Ios);
    config
        .facts
        .insert("has_user_accounts".to_string(), serde_json::json!(false));

    let project = load(&dir, &config);
    // The inference still runs and still says true.
    assert_eq!(
        project.derived_facts.get("has_user_accounts"),
        Some(&serde_json::json!(true))
    );
    // The answer wins where it counts.
    assert_eq!(
        project.fact("has_user_accounts", &config),
        Some(&serde_json::json!(false)),
        "a person who says the sign up code is dead is right, and the checker is not"
    );
    assert!(
        !project
            .assumed_facts
            .contains(&"has_user_accounts".to_string()),
        "a fact somebody answered is not an assumption"
    );
}

#[test]
fn the_plan_asks_only_about_facts_that_block_a_rule() {
    let dir = scratch("plan-scope", &[("Info.plist", PLIST)]);
    let config = PekoConfig::new(Platform::Ios);
    let project = load(&dir, &config);
    let db = database();
    let plan = plan::plan(&project, &config, &db);

    assert!(
        !plan.questions.is_empty(),
        "a project that declares nothing must be asked something"
    );
    // The vocabulary holds seventy facts, and most rules never read most of
    // them. Asking about all of them is the thing this module exists to
    // avoid, so the questionnaire must be a small part of the whole.
    let vocabulary = peko_rules::facts::declared().count();
    assert!(
        plan.questions.len() * 3 < vocabulary,
        "asked {} of {vocabulary} facts, which is not scoped to the project",
        plan.questions.len()
    );
    for question in &plan.questions {
        assert!(
            !question.blocks.is_empty(),
            "{} is asked but no rule waits on it",
            question.fact
        );
        assert!(
            peko_rules::facts::lookup(&question.fact).is_some(),
            "{} is not in the fact vocabulary",
            question.fact
        );
    }
    assert!(plan.decided > 0, "some rules must decide without help");
    assert!(!plan.complete(), "an empty config cannot be complete");
}

#[test]
fn evidence_in_the_code_shortens_the_questionnaire() {
    // This is the whole point of inference. The two projects declare the same
    // thing, which is nothing. One of them has code to read.
    let bare = scratch("short-bare", &[("Info.plist", PLIST)]);
    let coded = scratch(
        "short-coded",
        &[
            ("Info.plist", PLIST),
            (
                "Sources/App.swift",
                "func signUp() {}\nfunc createPost() {}\nlet birthDate = Date()\n",
            ),
        ],
    );
    let config = PekoConfig::new(Platform::Ios);
    let db = database();
    let before = plan::plan(&load(&bare, &config), &config, &db);
    let after = plan::plan(&load(&coded, &config), &config, &db);

    assert!(
        after.questions.len() < before.questions.len(),
        "code answered nothing: {} questions before and {} after",
        before.questions.len(),
        after.questions.len()
    );
    assert!(
        after.decided > before.decided,
        "an answered fact must let more rules decide"
    );
    let asked: Vec<&str> = after
        .questions
        .iter()
        .map(|question| question.fact.as_str())
        .collect();
    for settled in ["has_user_accounts", "has_user_generated_content"] {
        assert!(
            !asked.contains(&settled),
            "{settled} was read from the code and must not be asked again"
        );
    }
}

#[test]
fn answering_every_question_makes_the_plan_complete() {
    let dir = scratch("plan-complete", &[("Info.plist", PLIST)]);
    let mut config = PekoConfig::new(Platform::Ios);
    let db = database();

    let first = plan::plan(&load(&dir, &config), &config, &db);
    assert!(!first.complete());

    // Answer each question with a value of the shape it asked for. The point
    // is not the values. It is that the second pass has nothing left to ask.
    for question in &first.questions {
        let value = match question.shape.as_str() {
            "boolean" => serde_json::json!(false),
            "integer" => serde_json::json!(0),
            "string[]" => serde_json::json!(["us"]),
            _ => serde_json::json!("none"),
        };
        config.facts.insert(question.fact.clone(), value);
    }

    let second = plan::plan(&load(&dir, &config), &config, &db);
    assert!(
        second.complete(),
        "still asking about {:?}",
        second
            .questions
            .iter()
            .map(|question| question.fact.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        second.decided >= first.decided,
        "answering a fact must never make fewer rules decide"
    );
}

#[test]
fn the_config_block_names_every_answer_and_every_question() {
    let dir = scratch(
        "block",
        &[
            ("Info.plist", PLIST),
            ("Sources/Login.swift", "func signUp() {}\n"),
        ],
    );
    let config = PekoConfig::new(Platform::Ios);
    let project = load(&dir, &config);
    let plan = plan::plan(&project, &config, &database());
    let block = plan.config_block();
    let object = block.as_object().expect("the block is an object");

    for question in &plan.questions {
        assert_eq!(
            object.get(&question.fact),
            Some(&serde_json::Value::Null),
            "{} is unanswered, so the file must carry it as null",
            question.fact
        );
    }
    assert_eq!(
        object.get("has_user_accounts"),
        Some(&serde_json::json!(true)),
        "an inferred answer is written out, so a person can see it and change it"
    );
}

#[test]
fn an_inference_is_marked_apart_from_a_default() {
    // Both fill a gap, and only one has evidence behind it. A reader who
    // cannot tell them apart cannot tell which guesses to check.
    let dir = scratch(
        "sources",
        &[
            ("Info.plist", PLIST),
            ("Sources/Login.swift", "func signUp() {}\n"),
        ],
    );
    let config = PekoConfig::new(Platform::Ios);
    let project = load(&dir, &config);
    let plan = plan::plan(&project, &config, &database());

    let find = |name: &str| {
        plan.answered
            .iter()
            .find(|answer| answer.fact == name)
            .map(|answer| answer.source)
    };
    assert_eq!(find("has_user_accounts"), Some(Source::Inferred));
    assert!(
        plan.answered
            .iter()
            .any(|answer| answer.source == Source::Default),
        "the vocabulary carries defaults, and the plan must show them as defaults"
    );
    assert!(
        !plan
            .answered
            .iter()
            .any(|answer| answer.source == Source::Declared),
        "nothing was declared here"
    );
}

#[test]
fn a_privacy_policy_in_the_repository_answers_where_it_is() {
    let dir = scratch(
        "policy",
        &[
            ("Info.plist", PLIST),
            ("docs/PRIVACY.md", "We collect nothing.\n"),
        ],
    );
    let config = PekoConfig::new(Platform::Ios);
    let project = load(&dir, &config);
    let value = project
        .derived_facts
        .get("privacy_policy_document_path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    assert!(
        value.ends_with("PRIVACY.md"),
        "the path must point at the file, and it pointed at {value:?}"
    );
}
