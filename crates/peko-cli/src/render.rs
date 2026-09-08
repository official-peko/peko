//! Turning a report into something a person reads in a terminal.

use serde_json::Value;

/// The report as text, coloured when the terminal is one.
pub fn report(body: &Value) -> String {
    report_styled(body, crate::style::Style::detect())
}

/// The report, with the styling decided by the caller.
///
/// Laid out the way rustc lays out an error, because the people reading this
/// read that every day: what happened on one line, where underneath it, then
/// the detail and what to do. A wall of evenly indented text makes somebody
/// hunt for the file name, and the file name is what they came for.
#[allow(clippy::too_many_lines)]
pub fn report_styled(body: &Value, style: crate::style::Style) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let findings = body["findings"].as_array().cloned().unwrap_or_default();

    let counts = &body["summary"]["by_severity"];
    let errors = counts["error"].as_u64().unwrap_or(0);
    let warnings = counts["warning"].as_u64().unwrap_or(0);
    let infos = counts["info"].as_u64().unwrap_or(0);

    // The counts read as a sentence rather than a table, and the ones that are
    // zero are still shown. "1 error" beside nothing else invites the question
    // of whether the others were checked.
    let _ = writeln!(
        out,
        "{} findings: {} error, {} warning, {} info",
        findings.len(),
        style.severity_count("error", errors),
        style.severity_count("warning", warnings),
        style.severity_count("info", infos),
    );

    if findings.is_empty() {
        let _ = writeln!(out, "\n{}", style.fix("Nothing to fix."));
    }

    let _ = writeln!(out);
    for finding in &findings {
        let severity = finding["severity"].as_str().unwrap_or("info");
        let overridden = if finding["overridden"].as_bool().unwrap_or(false) {
            style.dim(" (acknowledged)")
        } else {
            String::new()
        };

        // severity, then the rule, then what it is. The rule id is dim because
        // it is a reference to look up, not the thing being said.
        let _ = writeln!(
            out,
            "{}{overridden}  {}  {}",
            style.severity(severity),
            style.dim(finding["rule_id"].as_str().unwrap_or("")),
            style.bold(finding["title"].as_str().unwrap_or("")),
        );

        if let Some(location) = finding["location"].as_object() {
            let line = location
                .get("line_start")
                .and_then(serde_json::Value::as_u64)
                .map_or_else(String::new, |line| format!(":{line}"));
            let file = location
                .get("file")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            // The arrow is rustc's, and an editor or a terminal that links
            // file paths picks this shape up.
            let _ = writeln!(
                out,
                "  {} {}",
                style.dim("-->"),
                style.place(&format!("{file}{line}"))
            );
        }
        if let Some(message) = finding["message"].as_str() {
            for line in wrap(message, 76) {
                let _ = writeln!(out, "      {line}");
            }
        }
        if let Some(fix) = finding["remediation"]["summary"].as_str() {
            let wrapped = wrap(fix, 72);
            for (index, line) in wrapped.iter().enumerate() {
                if index == 0 {
                    let _ = writeln!(out, "      {} {line}", style.fix("fix:"));
                } else {
                    let _ = writeln!(out, "           {line}");
                }
            }
        }
        let _ = writeln!(out);
    }

    // What the run assumed goes at the end whether or not anything was
    // found. A clean report resting on a guess is the case where the reader
    // most needs to know which guess.
    if let Some(assumed) = body["coverage"]["assumed_facts"].as_array() {
        if !assumed.is_empty() {
            let names: Vec<&str> = assumed.iter().filter_map(Value::as_str).collect();
            let _ = writeln!(
                out,
                "{}\nAnswer any of these in .pekorc.json.",
                style.dim(&format!(
                    "Assumed, because nobody answered: {}.",
                    names.join(", ")
                ))
            );
        }
    }
    out
}

/// Break a paragraph at a width, on spaces.
///
/// A message that runs past the terminal wraps wherever the terminal decides,
/// which is mid word and hard against the left margin, and the indent that
/// grouped it with its finding is lost.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// A rule listing as a table.
pub fn rule_list(body: &Value) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let rules = body["rules"].as_array().cloned().unwrap_or_default();
    let _ = writeln!(
        out,
        "{} rules, database {}\n",
        rules.len(),
        body["database_version"].as_str().unwrap_or("unknown")
    );
    for rule in &rules {
        let _ = writeln!(
            out,
            "{:<18} {:<8} {}",
            rule["rule_id"].as_str().unwrap_or(""),
            rule["severity"].as_str().unwrap_or(""),
            rule["title"].as_str().unwrap_or(""),
        );
    }
    out
}

/// What the process exits with.
///
/// A finding a person acknowledged never fails a build. That is the whole
/// point of acknowledging it, and a command that failed anyway would push
/// people to delete the rule instead.
pub fn exit_code(body: &Value, fail_on: &str) -> i32 {
    let rank = |severity: &str| match severity {
        "error" => 3,
        "warning" => 2,
        "info" => 1,
        _ => 0,
    };
    let threshold = rank(fail_on);
    if threshold == 0 {
        return 0;
    }
    let failing = body["findings"].as_array().is_some_and(|findings| {
        findings.iter().any(|finding| {
            !finding["overridden"].as_bool().unwrap_or(false)
                && rank(finding["severity"].as_str().unwrap_or("info")) >= threshold
        })
    });
    i32::from(failing)
}

/// Say which facts stopped a rule deciding, and whether that is a failure.
///
/// A rule that waits on an unanswered fact reports nothing. A report built on
/// that covers fewer rules than it appears to, so a clean result under it is
/// not a pass. The default is to fail, and `--allow-undecided` takes that
/// back for somebody who knows what they are giving up.
pub fn unanswered(body: &Value) -> String {
    use std::fmt::Write as _;

    let questions = match body["unanswered_facts"].as_array() {
        Some(list) if !list.is_empty() => list,
        _ => return String::new(),
    };
    let mut out = format!(
        "\n{} facts have no answer, so some rules did not run:\n",
        questions.len()
    );
    for question in questions {
        let name = question["fact"].as_str().unwrap_or("");
        let blocks = question["blocks"].as_array().map_or(0, Vec::len);
        let rules = if blocks == 1 { "rule" } else { "rules" };
        let _ = writeln!(out, "  {name} ({blocks} {rules})");
    }
    out.push_str("Run `peko facts --write` to fill in what the code answers.\n");
    out
}

/// True when the report leaves a rule waiting on an answer.
pub fn has_unanswered(body: &Value) -> bool {
    body["unanswered_facts"]
        .as_array()
        .is_some_and(|list| !list.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body(findings: &Value) -> Value {
        json!({
            "summary": {"by_severity": {"error": 1, "warning": 0, "info": 0}},
            "findings": findings.clone(),
            "coverage": {"assumed_facts": []},
        })
    }

    /// The counts come from the report and not from counting the list, so a
    /// name that drifts has to fail here rather than print zeroes beside a
    /// list of findings.
    #[test]
    fn the_counts_read_off_the_report() {
        let text = report(&body(&json!([{"severity": "error", "overridden": false}])));
        assert!(text.contains("1 error"), "{text}");
    }

    #[test]
    fn a_clean_report_says_so() {
        let text = report(&body(&json!([])));
        assert!(text.contains("Nothing to fix"), "{text}");
    }

    #[test]
    fn a_finding_names_the_rule_the_file_and_the_fix() {
        let text = report(&body(&json!([{
            "rule_id": "AAPL-API-001",
            "severity": "error",
            "title": "UIWebView is removed",
            "message": "UIWebView matches at line 24",
            "location": {"file": "App/View.swift", "line_start": 24},
            "remediation": {"summary": "Use WKWebView."},
            "overridden": false,
        }])));
        assert!(text.contains("AAPL-API-001"), "{text}");
        assert!(text.contains("App/View.swift:24"), "{text}");
        assert!(text.contains("fix: Use WKWebView."), "{text}");
    }

    #[test]
    fn an_error_fails_the_run() {
        let value = body(&json!([{"severity": "error", "overridden": false}]));
        assert_eq!(exit_code(&value, "error"), 1);
    }

    /// Acknowledging a finding is the whole point of acknowledging it. A
    /// command that failed anyway would push people to delete the rule.
    #[test]
    fn an_acknowledged_finding_does_not_fail_the_run() {
        let value = body(&json!([{"severity": "error", "overridden": true}]));
        assert_eq!(exit_code(&value, "error"), 0);
    }

    #[test]
    fn the_threshold_decides_what_fails() {
        let value = body(&json!([{"severity": "warning", "overridden": false}]));
        assert_eq!(exit_code(&value, "error"), 0);
        assert_eq!(exit_code(&value, "warning"), 1);
        assert_eq!(exit_code(&value, "never"), 0);
    }

    #[test]
    fn the_report_names_what_the_run_assumed() {
        let mut value = body(&json!([]));
        value["coverage"]["assumed_facts"] = json!(["mac_app_store", "shows_ads"]);
        let text = report_styled(&value, crate::style::Style::plain());
        assert!(text.contains("mac_app_store"), "{text}");
        assert!(text.contains(".pekorc.json"), "{text}");
    }
}

#[cfg(test)]
mod unanswered_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_report_with_every_fact_answered_says_nothing() {
        assert_eq!(unanswered(&json!({"findings": []})), "");
        assert_eq!(unanswered(&json!({"unanswered_facts": []})), "");
        assert!(!has_unanswered(&json!({"unanswered_facts": []})));
    }

    #[test]
    fn an_unanswered_fact_is_named_with_what_waits_on_it() {
        let body = json!({
            "unanswered_facts": [
                {"fact": "kids_category", "blocks": ["AAPL-MINOR-001", "AAPL-MINOR-002"]},
                {"fact": "distributes_in", "blocks": ["GDPR-001"]}
            ]
        });
        let text = unanswered(&body);
        assert!(text.contains("2 facts have no answer"));
        assert!(text.contains("kids_category (2 rules)"));
        // One rule is one rule. A count that reads "1 rules" looks like a bug
        // to the person reading it, whatever the code is doing.
        assert!(text.contains("distributes_in (1 rule)"), "{text}");
        assert!(text.contains("peko facts --write"));
        assert!(has_unanswered(&body));
    }

    #[test]
    fn an_unanswered_fact_does_not_change_the_severity_exit_code() {
        // The two are separate reasons to fail. `exit_code` answers the
        // severity question only, and the caller adds the other.
        let clean = json!({"findings": [], "unanswered_facts": [{"fact": "x", "blocks": ["R"]}]});
        assert_eq!(exit_code(&clean, "error"), 0);
        assert!(has_unanswered(&clean));
    }
}

/// Print what an audit would cost, and what stops it.
///
/// The price comes before the blockers. A reader who sees a wall of problems
/// first never gets to the number they asked for.
pub fn estimate(body: &Value) -> String {
    estimate_styled(body, crate::style::Style::detect())
}

/// What a run would read, and what it costs the person asking.
///
/// The cost is an audit out of the month's allowance, not a sum of money. The
/// dollars in the estimate are what the model costs the server, and a
/// subscriber neither pays them nor can change them. Leading with a price
/// invited the reasonable question of whether they were about to be charged
/// it, and the answer is no.
pub fn estimate_styled(body: &Value, style: crate::style::Style) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    let allowance = &body["allowance"];
    let included = allowance["audits_included"].as_u64().unwrap_or(0);
    if included > 0 {
        let used = allowance["audits_used"].as_u64().unwrap_or(0);
        let left = included.saturating_sub(used);
        let _ = writeln!(
            out,
            "This run uses one audit. {} of {included} left this month.",
            style.bold(&left.to_string())
        );
        if let Some(resets) = allowance["resets_on"].as_str().filter(|s| !s.is_empty()) {
            let _ = writeln!(out, "{}", style.dim(&format!("The count resets on {resets}.")));
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(
        out,
        "{}",
        style.dim(body["summary"].as_str().unwrap_or("no estimate"))
    );

    let rules = body["rules"].as_array().cloned().unwrap_or_default();
    if !rules.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "What it would read:");
        for rule in rules.iter().take(10) {
            // Files rather than fractions of a cent. What a rule reads is the
            // thing somebody can act on by narrowing the project.
            let files = rule["files"].as_array().map_or(0, Vec::len);
            let plural = if files == 1 { "file" } else { "files" };
            let _ = writeln!(
                out,
                "  {}  {files} {plural}",
                style.dim(rule["rule_id"].as_str().unwrap_or(""))
            );
        }
        if rules.len() > 10 {
            let _ = writeln!(out, "  and {} more rules", rules.len() - 10);
        }
    }

    let cached = body["cached"].as_array().map_or(0, Vec::len);
    if cached > 0 {
        let _ = writeln!(
            out,
            "\n{cached} rules are answered already and cost nothing."
        );
    }

    let blockers = body["blockers"].as_array().cloned().unwrap_or_default();
    if !blockers.is_empty() {
        let _ = writeln!(out, "\nThis will not run yet:");
        for blocker in &blockers {
            let _ = writeln!(out, "  {}", blocker["message"].as_str().unwrap_or(""));
        }
    }
    out
}

#[cfg(test)]
mod estimate_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_price_comes_before_the_problems() {
        let body = json!({
            "summary": "4 rules would read your code, about $0.42",
            "rules": [],
            "cached": [],
            "blockers": [{"reason": "lint_failing", "message": "The free checks report 2 errors."}],
        });
        let text = estimate_styled(&body, crate::style::Style::plain());
        let price = text.find("$0.42").expect("the price is printed");
        let problem = text.find("free checks").expect("the blocker is printed");
        assert!(
            price < problem,
            "the problems came before the price:\n{text}"
        );
    }

    #[test]
    fn a_clean_estimate_lists_no_problems() {
        let body = json!({
            "summary": "1 rule, about $0.10",
            "rules": [{"rule_id": "AAPL-PRIV-001", "estimated_cost_usd": 0.1, "files": ["a.swift"]}],
            "cached": [],
            "blockers": [],
        });
        let text = estimate_styled(&body, crate::style::Style::plain());
        assert!(text.contains("AAPL-PRIV-001"));
        assert!(!text.contains("will not run"));
    }

    #[test]
    fn a_long_list_is_cut_and_says_so() {
        let rules: Vec<_> = (0..25)
            .map(|index| json!({"rule_id": format!("R-{index}"), "estimated_cost_usd": 0.01, "files": []}))
            .collect();
        let text = estimate_styled(
            &json!({"summary": "s", "rules": rules, "cached": [], "blockers": []}),
            crate::style::Style::plain(),
        );
        assert!(text.contains("and 15 more rules"), "{text}");
    }

    #[test]
    fn what_the_cache_already_holds_is_named() {
        let text = estimate_styled(&json!({
            "summary": "s", "rules": [], "cached": ["A", "B"], "blockers": []
        }), crate::style::Style::plain());
        assert!(text.contains("2 rules are answered already"), "{text}");
    }
}

#[cfg(test)]
mod colour_tests {
    use super::report_styled;
    use crate::style::Style;
    use serde_json::json;

    fn sample() -> serde_json::Value {
        json!({
            "summary": { "by_severity": { "error": 1, "warning": 0, "info": 0 } },
            "findings": [{
                "rule_id": "AAPL-API-001",
                "title": "A removed API is still called",
                "severity": "error",
                "overridden": false,
                "message": "UIWebView is called in App/View.swift",
                "location": { "file": "App/View.swift", "line_start": 24 },
                "remediation": { "summary": "Use WKWebView." }
            }],
            "coverage": { "assumed_facts": [] }
        })
    }

    /// A report read by a script must carry no escape codes.
    ///
    /// This output is piped into grep and awk as often as it is read, and a
    /// colour code in the middle of a rule id turns a working pipeline into a
    /// puzzle.
    #[test]
    fn plain_output_holds_no_escape_codes() {
        let text = report_styled(&sample(), Style::plain());
        assert!(
            !text.contains('\u{1b}'),
            "a plain report must be plain: {text:?}"
        );
        assert!(text.contains("error  AAPL-API-001"));
        assert!(text.contains("--> App/View.swift:24"));
        assert!(text.contains("fix: Use WKWebView."));
    }

    /// The words carry the meaning, and the colour only repeats it.
    ///
    /// A reader who cannot tell red from amber, or who is reading a
    /// transcript, still has to be able to work out which findings block a
    /// release.
    #[test]
    fn the_severity_is_written_out_whether_or_not_it_is_coloured() {
        for style in [Style::plain(), Style::forced()] {
            let text = report_styled(&sample(), style);
            let stripped: String = strip(&text);
            assert!(
                stripped.contains("error  AAPL-API-001"),
                "the severity has to be a word, not only a colour: {stripped}"
            );
            assert!(stripped.contains("App/View.swift:24"));
        }
    }

    /// A long message wraps rather than running off the terminal.
    #[test]
    fn a_long_message_is_wrapped() {
        let mut value = sample();
        value["findings"][0]["message"] = json!(
            "A very long message that goes on well past the width of any \
             sensible terminal window and would otherwise wrap wherever the \
             terminal decided, which is mid word and hard against the margin."
        );
        let text = report_styled(&value, Style::plain());
        for line in text.lines() {
            assert!(
                line.chars().count() <= 90,
                "a line ran past the width: {line:?}"
            );
        }
    }

    fn strip(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }
}

#[cfg(test)]
mod allowance_tests {
    use super::estimate_styled;
    use crate::style::Style;
    use serde_json::json;

    fn body() -> serde_json::Value {
        json!({
            "summary": "12 rules would read your code, about $1.20 with the model",
            "rules": [{"rule_id": "app-store-5.1.1", "files": ["a", "b"]}],
            "cached": [],
            "allowance": {"audits_used": 6, "audits_included": 25, "resets_on": "2026-10-01"},
        })
    }

    /// The first line answers the question somebody actually has, which is
    /// what this run costs them. A subscriber pays no dollars per run.
    #[test]
    fn the_count_comes_before_the_dollars() {
        let out = estimate_styled(&body(), Style::plain());
        let counts = out.find("19 of 25").expect("the audits left are named");
        let dollars = out.find("$1.20").expect("the model cost is still shown");
        assert!(
            counts < dollars,
            "the price led the estimate and read as a charge:\n{out}"
        );
        assert!(out.contains("uses one audit"));
        assert!(out.contains("resets on 2026-10-01"));
    }

    /// A per-rule dollar figure is a fraction of a cent nobody can act on.
    /// What a rule reads is the thing narrowing the project changes.
    #[test]
    fn a_rule_line_names_files_rather_than_cents() {
        let out = estimate_styled(&body(), Style::plain());
        assert!(out.contains("app-store-5.1.1  2 files"), "{out}");
    }

    /// A server too old to send the allowance, or one with no database,
    /// still has to print something.
    #[test]
    fn a_missing_allowance_drops_the_line_rather_than_showing_zero() {
        let mut body = body();
        body["allowance"] = json!(null);
        let out = estimate_styled(&body, Style::plain());
        assert!(!out.contains("of 0"), "{out}");
        assert!(out.contains("12 rules would read your code"));
    }
}
