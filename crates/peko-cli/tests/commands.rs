//! The commands, against a real HTTP server.
//!
//! `peko-cli/src/main.rs` measured zero percent covered. Every command spoke
//! HTTP and lived in a binary, so nothing could reach one. The commands moved
//! into a library and these run them against a socket that answers.
//!
//! What matters here is the exit code and what reaches the wire. A customer
//! puts `peko lint` in a pipeline, and the number it returns decides whether
//! their release goes out.

mod support;

use support::{lint_answer, project, Server};

/// Give this project's key variable a value, then run the body.
///
/// `.pekorc.json` names the variable and never holds the key, so a test has
/// to set it. Each project uses its own variable name, because an environment
/// variable is global to the process and these tests run beside each other.
fn with_key<T>(name: &str, body: impl FnOnce() -> T) -> T {
    std::env::set_var(support::key_var(name), "peko_testkey");
    body()
}

const SOURCE: &str = "import UIKit\nclass V: UIWebView {}\n";
const PLIST: &str = r#"<?xml version="1.0"?><plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.example.t</string></dict></plist>"#;

fn files() -> Vec<(&'static str, &'static str)> {
    vec![("App/View.swift", SOURCE), ("App/Info.plist", PLIST)]
}

// ---------------------------------------------------------------------------
// lint
// ---------------------------------------------------------------------------

#[test]
fn lint_sends_the_files_and_fails_on_an_error_finding() {
    let server = Server::start(vec![(200, lint_answer())]);
    let root = project("lint", &server.url(), &files());
    let key = "lint";

    let code = with_key(key, || {
        peko_cli::lint(&root, true, "HEAD", None, false, "error", true).expect("lint runs")
    });
    assert_eq!(code, 1, "an error finding must fail the run");

    let seen = server.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert!(seen[0].path.ends_with("/lint"), "{}", seen[0].path);
    assert_eq!(
        seen[0].authorization.as_deref(),
        Some("Bearer peko_testkey"),
        "the key must travel as a bearer token"
    );

    // The source has to reach the server, or the check reads nothing.
    let body = seen[0].json();
    let sources = body["files"]["changed_sources"]
        .as_array()
        .expect("changed_sources");
    assert!(!sources.is_empty(), "no source was sent: {body}");
}

#[test]
fn lint_passes_when_nothing_is_wrong() {
    let clean = serde_json::json!({
        "tier": "lint",
        "findings": [],
        "summary": {"by_severity": {"error": 0, "warning": 0, "info": 0}},
        "requests_remaining_today": 99,
        "unanswered_facts": []
    })
    .to_string();
    let server = Server::start(vec![(200, clean)]);
    let root = project("lint-clean", &server.url(), &files());
    let key = "lint-clean";

    let code = with_key(key, || {
        peko_cli::lint(&root, true, "HEAD", None, false, "error", false).expect("runs")
    });
    assert_eq!(code, 0);
}

#[test]
fn lint_fails_on_an_unanswered_fact_unless_told_otherwise() {
    // A rule that waits on an answer reports nothing, and nothing reads like
    // a pass. The default is to fail.
    let quiet = serde_json::json!({
        "tier": "lint",
        "findings": [],
        "summary": {"by_severity": {"error": 0, "warning": 0, "info": 0}},
        "requests_remaining_today": 99,
        "unanswered_facts": [{"fact": "kids_category", "blocks": ["AAPL-MINOR-001"]}]
    })
    .to_string();

    let server = Server::start(vec![(200, quiet.clone())]);
    let root = project("lint-undecided", &server.url(), &files());
    let key = "lint-undecided";
    let strict = with_key(key, || {
        peko_cli::lint(&root, true, "HEAD", None, false, "error", false).expect("runs")
    });
    assert_eq!(strict, 1, "an unanswered fact must fail by default");

    let server = Server::start(vec![(200, quiet)]);
    let root = project("lint-allowed", &server.url(), &files());
    let key = "lint-allowed";
    let relaxed = with_key(key, || {
        peko_cli::lint(&root, true, "HEAD", None, false, "error", true).expect("runs")
    });
    assert_eq!(relaxed, 0, "--allow-undecided must take the failure back");
}

#[test]
fn lint_turns_a_server_error_into_the_message_a_person_reads() {
    let refusal = serde_json::json!({
        "error": {"code": "rate_limited", "message": "The plan allows 100 of these a day."}
    })
    .to_string();
    let server = Server::start(vec![(429, refusal)]);
    let root = project("lint-refused", &server.url(), &files());
    let key = "lint-refused";

    let error = with_key(key, || {
        peko_cli::lint(&root, true, "HEAD", None, false, "error", false).expect_err("must fail")
    });
    assert!(
        error.to_string().contains("100 of these a day"),
        "the server's own sentence must reach the person: {error}"
    );
}

#[test]
fn lint_without_a_key_runs_here_and_sends_nothing() {
    // This test used to assert that a missing key was an error. It is not any
    // more, and the change is the point rather than a way to make a test pass.
    // The mechanical tier reads files and calls no model, so a first run needs
    // no key and no server, and neither exists for somebody who has not signed
    // up yet.
    //
    // The half worth keeping is the second assertion. A local run must reach
    // no server at all, or a project sits on somebody's disk one moment and on
    // a network the next.
    let server = Server::start(vec![(200, lint_answer())]);
    let root = project("lint-nokey", &server.url(), &files());
    std::env::remove_var(support::key_var("lint-nokey"));

    let code = peko_cli::lint(&root, true, "HEAD", None, false, "error", false)
        .expect("a run with no key still works");
    assert_eq!(code, 1, "the fixture holds an error finding");
    assert_eq!(server.hits(), 0, "a local run reached the server");
}

// ---------------------------------------------------------------------------
// audit
// ---------------------------------------------------------------------------

fn estimate_answer(cost: f64, blockers: &serde_json::Value) -> String {
    serde_json::json!({
        "model": "claude-sonnet-5",
        "rules": [{"rule_id": "AAPL-PRIV-001", "title": "a rule",
                   "files": ["App/View.swift"], "total_chars": 100,
                   "dropped": 0, "estimated_cost_usd": cost}],
        "skipped": [],
        "cached": [],
        "estimated_cost_usd": cost,
        "blockers": blockers,
        "summary": format!("1 rule would read your code, about ${cost:.2}")
    })
    .to_string()
}

#[test]
fn audit_without_yes_prints_the_price_and_spends_nothing() {
    // Nobody finds out what the expensive tier costs by being charged for it.
    let server = Server::start(vec![(200, estimate_answer(0.42, &serde_json::json!([])))]);
    let root = project("audit-quote", &server.url(), &files());
    let key = "audit-quote";

    let code = with_key(key, || {
        peko_cli::audit(&root, false, None, false).expect("runs")
    });
    assert_eq!(code, 0, "a price with nothing blocking it is not a failure");

    let seen = server.requests();
    assert_eq!(seen.len(), 1, "only the estimate may be called");
    assert!(
        seen[0].path.ends_with("/audit/estimate"),
        "{}",
        seen[0].path
    );
}

#[test]
fn audit_without_yes_fails_when_something_blocks_it() {
    let blocked = estimate_answer(
        0.42,
        &serde_json::json!([{"reason": "lint_failing",
                            "message": "The free checks report 2 errors."}]),
    );
    let server = Server::start(vec![(200, blocked)]);
    let root = project("audit-blocked", &server.url(), &files());
    let key = "audit-blocked";

    let code = with_key(key, || {
        peko_cli::audit(&root, false, None, false).expect("runs")
    });
    assert_eq!(code, 1, "a blocker must fail the run");
}

#[test]
fn audit_with_yes_and_no_limit_refuses_before_it_sends_anything() {
    // A run with no cap has no answer to how much it cost.
    let server = Server::start(vec![(200, estimate_answer(0.42, &serde_json::json!([])))]);
    let root = project("audit-nolimit", &server.url(), &files());
    let key = "audit-nolimit";

    let error = with_key(key, || {
        peko_cli::audit(&root, true, None, false).expect_err("must refuse")
    });
    assert!(error.to_string().contains("--max-spend"), "{error}");
    assert_eq!(server.hits(), 1, "only the estimate may have been called");
}

#[test]
fn audit_refuses_a_limit_under_the_estimate_without_starting_a_job() {
    let server = Server::start(vec![(200, estimate_answer(2.00, &serde_json::json!([])))]);
    let root = project("audit-cheap", &server.url(), &files());
    let key = "audit-cheap";

    let error = with_key(key, || {
        peko_cli::audit(&root, true, Some(0.50), false).expect_err("must refuse")
    });
    let text = error.to_string();
    assert!(text.contains("$2.00") && text.contains("$0.50"), "{text}");
    assert_eq!(server.hits(), 1, "the audit was started anyway");
}

#[test]
fn audit_starts_a_job_and_polls_until_it_finishes() {
    let started = serde_json::json!({
        "job_id": "11111111-1111-1111-1111-111111111111",
        "state": "running", "rules_total": 2,
        "estimated_cost_usd": 0.42, "requests_remaining_today": 4,
        "poll": "/v1/audit/11111111-1111-1111-1111-111111111111"
    })
    .to_string();
    let running = serde_json::json!({
        "job_id": "11111111-1111-1111-1111-111111111111",
        "state": "running", "rules_total": 2, "rules_done": 1,
        "spent_usd": 0.21, "report_available_for_minutes": 60
    })
    .to_string();
    let done = serde_json::json!({
        "job_id": "11111111-1111-1111-1111-111111111111",
        "state": "done", "rules_total": 2, "rules_done": 2,
        "spent_usd": 0.42, "report_available_for_minutes": 60,
        "report": {"tier": "audit", "findings": [],
                   "summary": {"by_severity": {"error": 0, "warning": 0, "info": 0}}}
    })
    .to_string();

    let server = Server::start(vec![
        (200, estimate_answer(0.42, &serde_json::json!([]))),
        (200, started),
        (200, running),
        (200, done),
    ]);
    let root = project("audit-job", &server.url(), &files());
    let key = "audit-job";

    let code = with_key(key, || {
        peko_cli::audit(&root, true, Some(5.0), false).expect("runs")
    });
    assert_eq!(code, 0, "a clean report must pass");

    let seen = server.requests();
    assert!(
        seen.len() >= 4,
        "estimate, start, then polls: {}",
        seen.len()
    );
    assert!(seen[1].path.ends_with("/audit"), "{}", seen[1].path);
    assert_eq!(seen[2].method, "GET", "the poll must be a GET");
    assert!(seen[2].path.contains("11111111"), "{}", seen[2].path);
}

#[test]
fn audit_reports_a_failed_job_as_an_error_and_not_a_pass() {
    let started = serde_json::json!({
        "job_id": "22222222-2222-2222-2222-222222222222",
        "state": "running", "rules_total": 1,
        "estimated_cost_usd": 0.1, "requests_remaining_today": 4,
        "poll": "/v1/audit/22222222-2222-2222-2222-222222222222"
    })
    .to_string();
    let failed = serde_json::json!({
        "job_id": "22222222-2222-2222-2222-222222222222",
        "state": "failed", "rules_total": 1, "rules_done": 0, "spent_usd": 0.0,
        "report_available_for_minutes": 60,
        "error": "The server running this audit stopped answering."
    })
    .to_string();

    let server = Server::start(vec![
        (200, estimate_answer(0.1, &serde_json::json!([]))),
        (200, started),
        (200, failed),
    ]);
    let root = project("audit-failed", &server.url(), &files());
    let key = "audit-failed";

    let error = with_key(key, || {
        peko_cli::audit(&root, true, Some(5.0), false).expect_err("must fail")
    });
    assert!(error.to_string().contains("stopped answering"), "{error}");
}

// ---------------------------------------------------------------------------
// facts, override, rules, status
// ---------------------------------------------------------------------------

#[test]
fn facts_writes_what_the_code_answered_and_leaves_the_rest_null() {
    let answer = serde_json::json!({
        "facts": {"has_user_accounts": true, "kids_category": null},
        "answered": [{"fact": "has_user_accounts", "value": true, "source": "inferred"}],
        "questions": [{"fact": "kids_category", "prompt": "Is it for children?",
                       "shape": "boolean", "blocks": ["AAPL-MINOR-001"]}],
        "complete": false,
        "decided": 12
    })
    .to_string();
    let server = Server::start(vec![(200, answer)]);
    let root = project("facts", &server.url(), &files());
    let key = "facts";

    let code = with_key(key, || peko_cli::facts(&root, true).expect("runs"));
    assert_eq!(code, 1, "a question left over must fail");

    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join(".pekorc.json")).expect("read"))
            .expect("parse");
    assert_eq!(written["facts"]["has_user_accounts"], true);
    assert!(
        written["facts"]["kids_category"].is_null(),
        "an unanswered fact must be written as null so the file carries its own list"
    );
}

#[test]
fn facts_without_write_changes_nothing_on_disk() {
    let answer = serde_json::json!({
        "facts": {"has_user_accounts": true},
        "answered": [{"fact": "has_user_accounts", "value": true, "source": "inferred"}],
        "questions": [], "complete": true, "decided": 12
    })
    .to_string();
    let server = Server::start(vec![(200, answer)]);
    let root = project("facts-dry", &server.url(), &files());
    let key = "facts-dry";
    let before = std::fs::read_to_string(root.join(".pekorc.json")).expect("read");

    let code = with_key(key, || peko_cli::facts(&root, false).expect("runs"));
    assert_eq!(code, 0, "nothing left to answer is a pass");
    assert_eq!(
        std::fs::read_to_string(root.join(".pekorc.json")).expect("read"),
        before
    );
}

#[test]
fn an_override_on_a_rule_that_cannot_be_overridden_is_refused() {
    // Apple rejects an upload holding UIWebView, so no reason a team writes
    // down changes what happens. Writing the override would say the finding
    // is handled and every later run would report it anyway.
    let answer = serde_json::json!({
        "rules": [{"rule_id": "AAPL-API-001", "overridable": false}]
    })
    .to_string();
    let server = Server::start(vec![(200, answer)]);
    let root = project("override-refused", &server.url(), &files());
    let key = "override-refused";

    let error = with_key(key, || {
        peko_cli::add_override(&root, "AAPL-API-001", "we know").expect_err("must refuse")
    });
    assert!(
        error.to_string().contains("cannot be overridden"),
        "{error}"
    );

    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join(".pekorc.json")).expect("read"))
            .expect("parse");
    assert!(
        written["overrides"]
            .as_array()
            .expect("overrides")
            .is_empty(),
        "a refused override reached the file"
    );
}

#[test]
fn an_override_on_a_rule_that_permits_one_is_written_with_its_reason() {
    let answer = serde_json::json!({
        "rules": [{"rule_id": "AAPL-PRIV-010", "overridable": true}]
    })
    .to_string();
    let server = Server::start(vec![(200, answer)]);
    let root = project("override-ok", &server.url(), &files());
    let key = "override-ok";

    let code = with_key(key, || {
        peko_cli::add_override(&root, "AAPL-PRIV-010", "shipped elsewhere").expect("runs")
    });
    assert_eq!(code, 0);

    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join(".pekorc.json")).expect("read"))
            .expect("parse");
    let overrides = written["overrides"].as_array().expect("overrides");
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0]["rule_id"], "AAPL-PRIV-010");
    assert_eq!(overrides[0]["reason"], "shipped elsewhere");
}

#[test]
fn status_reports_what_the_server_says_about_itself() {
    let health = serde_json::json!({
        "status": "healthy",
        "rule_database_version": "0.1.0",
        "interpretive_engine": "available",
        "durable_limits": true
    })
    .to_string();
    let server = Server::start(vec![(200, health)]);
    let root = project("status", &server.url(), &files());
    let key = "status";

    let code = with_key(key, || peko_cli::status(&root).expect("runs"));
    assert_eq!(code, 0);
    let seen = server.requests();
    assert!(seen[0].path.ends_with("/health"), "{}", seen[0].path);
}

#[test]
fn init_writes_a_config_and_then_leaves_an_existing_one_alone() {
    let server = Server::start(vec![(
        200,
        serde_json::json!({"facts": {}, "answered": [], "questions": [],
                           "complete": true, "decided": 0})
        .to_string(),
    )]);
    let key = "init";
    let root = std::env::temp_dir().join(format!("peko-cli-init-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("App")).expect("make it");
    std::fs::write(root.join("App/Info.plist"), PLIST).expect("write");

    with_key(key, || {
        peko_cli::init(&root, Some("ios")).expect("init runs")
    });
    assert!(root.join(".pekorc.json").exists(), "no config was written");

    // Point it at the server, then prove a second init does not overwrite.
    support::write_config_named(&root, &server.url(), &support::key_var(key));
    let before = std::fs::read_to_string(root.join(".pekorc.json")).expect("read");
    with_key(key, || peko_cli::init(&root, Some("ios")).expect("runs"));
    assert_eq!(
        std::fs::read_to_string(root.join(".pekorc.json")).expect("read"),
        before,
        "init overwrote a config that was already there"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn lint_runs_locally_when_there_is_no_key() {
    // A first run had to find a deployed server and an issued key before it
    // could show one finding, and neither exists for somebody who has not
    // signed up. The mechanical tier calls no model, so it needs neither.
    //
    // The server address here points at a port that answers nothing. If the
    // run reaches the network at all, it fails rather than passes quietly.
    let root = support::project(
        "localnokey",
        "http://127.0.0.1:1/v1",
        &[
            (
                "Info.plist",
                "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict>\
                 <key>CFBundleIdentifier</key><string>com.example.demo</string></dict></plist>",
            ),
            (
                "App/View.swift",
                "import UIKit\nclass V: UIViewController {\n  let web = UIWebView()\n}\n",
            ),
        ],
    );

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_peko"))
        .args(["lint", "--all"])
        .current_dir(&root)
        // The key variable is deliberately absent, so api_key() fails and the
        // run falls through to the local path.
        .env_remove(support::key_var("localnokey"))
        .output()
        .expect("the binary runs");
    let text = String::from_utf8_lossy(&output.stdout);

    assert!(
        text.contains("AAPL-API-001"),
        "the local run found no UIWebView:\n{text}"
    );
    assert!(
        text.contains("Checked on this machine"),
        "the report did not say where it ran:\n{text}"
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "an error finding must fail the run"
    );
}

#[test]
fn the_local_run_names_the_database_it_used() {
    // A finding nobody can trace to a database version cannot be reproduced.
    let root = support::project(
        "localversion",
        "http://127.0.0.1:1/v1",
        &[(
            "Info.plist",
            "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict/></plist>",
        )],
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_peko"))
        .args(["lint", "--all", "--json"])
        .current_dir(&root)
        .env_remove(support::key_var("localversion"))
        .output()
        .expect("the binary runs");
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("the local run answers json");
    assert!(
        report["rule_database_version"].is_string(),
        "no database version in the report"
    );
    assert_eq!(report["tier"], "lint");
}
