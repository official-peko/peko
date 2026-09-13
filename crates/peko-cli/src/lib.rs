//! `peko`, the client.
//!
//! It gathers files, sends them to the API, and prints what comes back. It
//! holds no rules and no analysis: the rule database is the product and it
//! lives on the server, so a rule fix reaches every caller the moment it
//! promotes rather than when they next upgrade this binary.
//!
//! Every command lives here rather than in the binary, so a test can call one
//! against a real HTTP server. While they lived in `main.rs` nothing outside
//! could reach them, and the file measured zero percent covered.

pub mod config;
pub mod gather;
pub mod local;
pub mod outcome;
pub mod release;
pub mod start;
pub mod render;
pub mod style;
pub mod telemetry;
pub mod update;

use anyhow::{Context, Result};
pub use config::Config;
use std::path::Path;

pub fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .context("failed to build the HTTP client")
}

/// Turn a server error envelope into a sentence.
///
/// The server sends a code and a message. The code is for a program, and the
/// message is the one a person reads, so that is the one printed.
pub fn describe(status: reqwest::StatusCode, body: &str) -> anyhow::Error {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(message) = value["error"]["message"].as_str() {
            return anyhow::anyhow!("{message}");
        }
    }
    anyhow::anyhow!("the server answered {status}")
}

/// Print a limit that money or time removes, rather than raise an error.
///
/// A 402 is not a failure. Nothing broke, nothing is wrong with the project,
/// and there is nothing to debug. Printing it as `Error:` next to a stack of
/// real errors tells somebody to go looking for a fault that is not there.
///
/// Returns true when the answer was one of those, so the caller stops without
/// raising. Exit code 0, because the command did what it was asked to do:
/// find out whether an audit could run.
pub fn print_limit(status: reqwest::StatusCode, body: &str) -> bool {
    if status != reqwest::StatusCode::PAYMENT_REQUIRED {
        return false;
    }
    let value: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let message = value["error"]["message"]
        .as_str()
        .unwrap_or("This plan does not include an audit.");

    eprintln!();
    eprintln!("The audit tier is not open on this account.");
    eprintln!();
    for line in wrap(message, 72) {
        eprintln!("  {line}");
    }
    eprintln!();
    eprintln!("  The mechanical lint is unaffected. `peko lint --all` still runs");
    eprintln!("  here, offline, and needs no account.");
    eprintln!();
    true
}

/// Break a paragraph at a width, on spaces.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
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

/// What one lint run was asked for.
///
/// These arrived as eight positional arguments, and a caller that swapped two
/// bools got a different run with no complaint from the compiler.
pub struct LintOptions<'a> {
    /// Check the whole project rather than what changed.
    pub all: bool,
    /// The commit to compare against.
    pub since: &'a str,
    /// Name the platform rather than read it from the config.
    pub platform: Option<&'a str>,
    /// Print the report as JSON.
    pub json: bool,
    /// Also write SARIF here, for Code Scanning.
    pub sarif: Option<&'a Path>,
    /// The severity that makes the command exit non zero.
    pub fail_on: &'a str,
    /// Report a pass even when a fact has no answer.
    pub allow_undecided: bool,
}

pub fn lint(root: &Path, options: &LintOptions<'_>) -> Result<i32> {
    let LintOptions {
        all,
        since,
        platform,
        json,
        sarif,
        fail_on,
        allow_undecided,
    } = *options;
    let mut config = Config::load(root)?;
    if let Some(named) = platform {
        config.platform = named.to_string();
    }

    // The mechanical tier reads files and calls no model, so it needs no key
    // and no server. A caller with a key still goes to the server, because the
    // rule database there is current without upgrading this binary. A caller
    // without one gets the same checks from the database compiled in.
    //
    // Without this, a first run needed a deployed server and an issued key,
    // and neither exists for somebody who has not signed up.
    let Ok(key) = config.api_key() else {
        return lint_locally(root, &config, options);
    };

    let changed = if all {
        Vec::new()
    } else {
        gather::changed_files(root, since)
    };
    let (files, skipped) = gather::collect(root, &changed);
    if files.changed_sources.is_empty() && !all {
        println!("Nothing changed since {since}. Pass --all to check the whole project.");
        return Ok(0);
    }
    for path in &skipped {
        eprintln!("peko: {path} is too large for a lint, and was left out");
    }

    let overrides = std::fs::read(root.join(config::FILE)).ok().map(|bytes| {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    });

    let body = serde_json::json!({
        "platform": config.platform,
        "files": files,
        "overrides": overrides,
    });

    let response = client()?
        .post(format!("{}/lint", config.api_url))
        .bearer_auth(key)
        .json(&body)
        .send()
        .context("the server did not answer")?;
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(describe(status, &text));
    }

    let report: serde_json::Value = serde_json::from_str(&text)?;
    if let Some(path) = sarif {
        write_sarif(&report, path)?;
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(render::exit_code(&report, fail_on));
    }
    print!("{}", render::report(&report));

    // A rule that waits on an unanswered fact reports nothing. A report built
    // on that is not a pass, so say what is missing and fail.
    print!("{}", render::unanswered(&report));
    if allow_undecided || !render::has_unanswered(&report) {
        return Ok(render::exit_code(&report, fail_on));
    }
    Ok(1)
}

/// Write the report as SARIF, so Code Scanning can put it on the diff.
///
/// A clean run writes the file too. Without one, Code Scanning keeps
/// yesterday's alerts open after the fix that closed them.
fn write_sarif(report: &serde_json::Value, path: &Path) -> Result<()> {
    let parsed: peko_report::Report = serde_json::from_value(report.clone())
        .context("the report does not match the schema SARIF is built from")?;
    let document = peko_report::sarif::render(&parsed);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to make {}", parent.display()))?;
        }
    }
    std::fs::write(path, serde_json::to_string_pretty(&document)? + "\n")
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Run the mechanical checks here, with the database compiled in.
fn lint_locally(root: &Path, config: &Config, options: &LintOptions<'_>) -> Result<i32> {
    let LintOptions {
        all,
        since,
        json,
        sarif,
        fail_on,
        allow_undecided,
        ..
    } = *options;
    if !all {
        // The server path sends only what changed. The local path reads the
        // whole project, because the engine walks the tree itself and a
        // partial read would report a pass on a file it never opened.
        let changed = gather::changed_files(root, since);
        if changed.is_empty() {
            println!("Nothing changed since {since}. Checking the whole project.");
        }
    }
    let (_, source) = update::database(Some(&config.api_url))?;
    let report = local::lint(root, &config.platform, Some(&config.api_url))?;
    if let Some(path) = sarif {
        write_sarif(&report, path)?;
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(render::exit_code(&report, fail_on));
    }
    print!("{}", render::report(&report));
    let origin = match source {
        update::Source::Embedded => "shipped with this binary",
        update::Source::Cache => "cached from an earlier fetch",
        update::Source::Fetched => "fetched and signature checked",
    };
    println!(
        "\nChecked on this machine with rule database {} ({origin}). The audit \
         tier needs a key: run `peko login`.",
        report["rule_database_version"]
            .as_str()
            .unwrap_or("unknown")
    );
    print!("{}", render::unanswered(&report));
    if allow_undecided || !render::has_unanswered(&report) {
        return Ok(render::exit_code(&report, fail_on));
    }
    Ok(1)
}

pub fn init(root: &Path, platform: Option<&str>) -> Result<i32> {
    let path = root.join(config::FILE);
    if path.exists() {
        // Every config written before cycles existed is missing the project
        // id, and without one the server bills each run on its own. Nobody
        // would think to run init again to fix a thing they never knew was
        // missing, so this is the one command that would plausibly be run and
        // it fills the gap rather than reporting it.
        match backfill_project(&path) {
            Ok(true) => {
                println!("{} is already here.", path.display());
                println!();
                println!("Added a project id, which is what groups a week of audits");
                println!("against this app into one. Commit it, so a teammate and CI");
                println!("land in the same week rather than each buying their own.");
                return Ok(0);
            }
            Ok(false) => {}
            Err(error) => eprintln!("peko: could not read {}: {error}", path.display()),
        }
        println!("{} is already here. Nothing changed.", path.display());
        println!("Run `peko facts` to fill in what is missing.");
        return Ok(0);
    }
    // A second config inside a project that already has one splits the
    // project in two, and the smaller half wins. Every command reads the file
    // beside it, so a run started here sees the files under here and the
    // answers written here, and reports on a fragment as though it were the
    // app. It looks like it worked: a config appears, facts are asked for,
    // and nothing says the real one is one directory up.
    if let Some(above) = config_above(root) {
        println!("{} already covers this directory.", above.display());
        println!();
        println!("A second one here would split the project in two, and a run");
        println!("started here would read only what is under here.");
        println!();
        println!(
            "Run peko from {} instead.",
            above.parent().unwrap_or(root).display()
        );
        println!("If this really is a project of its own, move it out first.");
        return Ok(1);
    }
    let platform = match platform {
        Some(named) => named.to_string(),
        None => config::detect_platform(root)?,
    };
    // Random, and committed. It only has to be unlike every other project's,
    // and a name taken from the directory would not be: half the apps in the
    // world live in a directory called app, and two of them under one account
    // would share a cycle and each get half a week.
    let project = uuid::Uuid::new_v4().to_string();
    let doc = serde_json::json!({
        "version": 1,
        "platform": platform,
        "api_key_env": "PEKO_API_KEY",
        "project": project,
        "facts": {},
        "overrides": [],
    });
    std::fs::write(&path, serde_json::to_string_pretty(&doc)? + "\n")?;
    println!("Wrote {}.", path.display());

    // No key is not a failure here. The lint needs no account, and this is
    // the first command anybody runs: telling them the server could not be
    // reached, when the server is fine and they simply have no key, makes the
    // tool look broken on the sentence after it says it worked.
    if Config::load(root).is_ok_and(|config| config.api_key().is_err()) {
        println!();
        println!("Some rules need answers the code cannot give, and filling those");
        println!("in needs a key. The lint does not, so start there:");
        println!();
        println!("  peko lint --all");
        println!();
        println!("Run `peko facts --write` once you have a key.");
        return Ok(0);
    }

    // A file with an empty facts block is not usable yet. Every rule that
    // needs an answer reports undecided, and undecided reads like a pass. So
    // fill in what the project answers for itself, and name the rest.
    match facts(root, true) {
        Ok(code) => Ok(code),
        Err(error) => {
            eprintln!("peko: could not fill in the facts: {error}");
            println!("Run `peko facts --write` to try again.");
            Ok(0)
        }
    }
}

/// Give an existing config a project id, if it has none.
///
/// Returns whether anything was written. The file is rewritten from the value
/// that was parsed out of it, so a key this build does not know about
/// survives: the config belongs to the project, and dropping a field because
/// this version had no name for it would be losing somebody else's data.
fn backfill_project(path: &Path) -> Result<bool> {
    let text = std::fs::read_to_string(path)?;
    let mut doc: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| anyhow::anyhow!("{} does not parse: {error}", path.display()))?;
    let Some(object) = doc.as_object_mut() else {
        return Ok(false);
    };
    // A present id is never replaced. Replacing one would end the week the
    // customer is in the middle of and charge them for the next run.
    if object.get("project").is_some_and(|value| value.is_string()) {
        return Ok(false);
    }
    object.insert(
        "project".to_string(),
        serde_json::Value::String(uuid::Uuid::new_v4().to_string()),
    );
    std::fs::write(path, serde_json::to_string_pretty(&doc)? + "\n")?;
    Ok(true)
}

/// The nearest `.pekorc.json` in a directory above this one.
///
/// `root` itself is not checked. The caller has already handled that, and it
/// means something different: a file here is the project, and a file above is
/// somebody standing in the wrong directory.
fn config_above(root: &Path) -> Option<std::path::PathBuf> {
    let start = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    start.ancestors().skip(1).find_map(|dir| {
        let candidate = dir.join(config::FILE);
        candidate.exists().then_some(candidate)
    })
}

/// Run the audit, or say what it would cost.
///
/// The estimate always runs first and it always costs nothing. `--yes` is the
/// only thing that spends money, and it needs a number with it.
pub fn audit(root: &Path, yes: bool, json: bool) -> Result<i32> {
    audit_run(root, yes, json, None)
}

/// Run an audit with a key that did not come from the environment.
///
/// The guided demo uses this. It fetches a key that can only run the sample
/// app, so nobody has to be handed a credential to paste, and the ordinary
/// path stays the only one that reads `PEKO_API_KEY`.
///
/// # Errors
///
/// The same as `audit`.
pub fn audit_with_key(root: &Path, key: &str) -> Result<i32> {
    audit_run(root, true, false, Some(key.to_string()))
}

fn audit_run(root: &Path, yes: bool, json: bool, given: Option<String>) -> Result<i32> {
    let config = Config::load(root)?;
    let key = match given {
        Some(key) => key,
        None => config.api_key()?,
    };
    let (files, skipped) = gather::collect(root, &[]);
    for path in &skipped {
        eprintln!("peko: {path} is too large for an audit, and was left out");
    }
    let overrides = std::fs::read(root.join(config::FILE)).ok().map(|bytes| {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    });

    let estimate: serde_json::Value = {
        let body = serde_json::json!({
            "platform": config.platform,
            "files": files,
            "overrides": overrides,
            // Sent so the answer can say whether this run is already paid
            // for. The estimate only reads the cycle, it never joins one, so
            // asking twice costs nothing.
            "project_id": config.project,
        });
        let response = client()?
            .post(format!("{}/audit/estimate", config.api_url))
            .bearer_auth(&key)
            .json(&body)
            .send()
            .context("the server did not answer")?;
        let status = response.status();
        let text = response.text().unwrap_or_default();
        if !status.is_success() {
            // A tier that does not include this is not a fault to report.
            if print_limit(status, &text) {
                return Ok(0);
            }
            return Err(describe(status, &text));
        }
        serde_json::from_str(&text)?
    };

    let blockers = estimate["blockers"].as_array().cloned().unwrap_or_default();

    if !yes {
        print!("{}", render::estimate(&estimate));
        println!();
        if blockers.is_empty() {
            println!("Nothing is stopping this run.");
            println!("To run it:  peko audit --yes");
        } else {
            println!("Fix the above first, then run it with --yes.");
        }
        // Showing what a run would use is not a failure, and a blocker is.
        return Ok(i32::from(!blockers.is_empty()));
    }

    // Nothing here names a cap. The plan sets what one run may cost and the
    // server holds it. A subscriber pays a flat price each month, so the
    // dollars are what the model costs us rather than anything they owe, and
    // a flag asking them to name one invited the reasonable question of
    // whether they were about to be charged it.
    let body = serde_json::json!({
        "platform": config.platform,
        "files": files,
        "overrides": overrides,
        "confirm": true,
        "project_id": config.project,
    });
    let response = client()?
        .post(format!("{}/audit", config.api_url))
        .bearer_auth(&key)
        .json(&body)
        .send()
        .context("the server did not answer")?;
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        // The month can run out between the estimate and the start, so this
        // path meets the same wall the estimate does.
        if print_limit(status, &text) {
            return Ok(0);
        }
        return Err(describe(status, &text));
    }
    let started: serde_json::Value = serde_json::from_str(&text)?;
    let job = started["job_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("the server did not name a job"))?
        .to_string();
    let total = started["rules_total"].as_u64().unwrap_or(0);

    // An audit takes minutes, so the server hands back a job and this polls
    // it. A held request would be at the mercy of every proxy in between, and
    // when it broke there would be no way to ask how far it got.
    if !json {
        println!("Reading {total} rules. This takes a few minutes.");
    }
    let report = poll_audit(&config, &key, &job, json)?;

    remember_run(root, &job, &report);

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(render::exit_code(&report["report"], "error"));
    }
    print!("{}", render::report(&report["report"]));
    println!();
    Ok(render::exit_code(&report["report"], "error"))
}

/// Write down the audit that just finished.
///
/// So a rejection weeks later can be attached to the run that was supposed to
/// predict it. The rules it raised come along, because the server deletes the
/// job an hour after it finishes and nothing else remembers them.
fn remember_run(root: &Path, job: &str, report: &serde_json::Value) {
    let mut flagged: Vec<String> = report["report"]["findings"]
        .as_array()
        .map(|findings| {
            findings
                .iter()
                .filter_map(|finding| finding["rule_id"].as_str())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();
    flagged.sort();
    flagged.dedup();
    outcome::remember(
        root,
        &outcome::LastRun {
            job_id: job.to_string(),
            finished_on: chrono_date(),
            flagged,
        },
    );
}

/// Today, as a date. No time, because nothing here needs one and a timestamp
/// in a file somebody may commit is more precision than the question wants.
fn chrono_date() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// How often to ask the server how the audit is going.
///
/// Every second would be noise on a run that takes minutes, and every thirty
/// would leave somebody watching a still screen.
const POLL_SECONDS: u64 = 3;

/// Wait for an audit to finish, and print how far it has got.
pub fn poll_audit(config: &Config, key: &str, job: &str, quiet: bool) -> Result<serde_json::Value> {
    let mut last = 0u64;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(POLL_SECONDS));
        let response = client()?
            .get(format!("{}/audit/{job}", config.api_url))
            .bearer_auth(key)
            .send()
            .context("the server did not answer")?;
        let status = response.status();
        let text = response.text().unwrap_or_default();
        if !status.is_success() {
            return Err(describe(status, &text));
        }
        let job_body: serde_json::Value = serde_json::from_str(&text)?;

        match job_body["state"].as_str().unwrap_or("running") {
            "done" => return Ok(job_body),
            "failed" => {
                return Err(anyhow::anyhow!(
                    "{}",
                    job_body["error"]
                        .as_str()
                        .unwrap_or("the audit failed and said nothing")
                ));
            }
            _ => {
                let done = job_body["rules_done"].as_u64().unwrap_or(0);
                let total = job_body["rules_total"].as_u64().unwrap_or(0);
                if !quiet && done != last {
                    println!("  {done} of {total} rules");
                    last = done;
                }
            }
        }
    }
}

/// Ask the server what the project answers for itself.
///
/// The checker reads the code and settles what the code can settle. It never
/// answers a question with `false` on weak evidence, because a wrong `false`
/// makes a rule stay silent and nobody sees the finding. So a fact it cannot
/// settle stays a question for a person.
pub fn facts(root: &Path, write: bool) -> Result<i32> {
    let config = Config::load(root)?;
    let key = config.api_key()?;
    let (files, _) = gather::collect(root, &[]);

    let overrides = std::fs::read(root.join(config::FILE)).ok().map(|bytes| {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    });
    let body = serde_json::json!({
        "platform": config.platform,
        "files": files,
        "overrides": overrides,
    });

    let response = client()?
        .post(format!("{}/facts", config.api_url))
        .bearer_auth(key)
        .json(&body)
        .send()
        .context("the server did not answer")?;
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(describe(status, &text));
    }
    let answer: serde_json::Value = serde_json::from_str(&text)?;

    if write {
        let path = root.join(config::FILE);
        let mut doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
        // Keep every answer already in the file. The server sent them back
        // unchanged, but reading the file again means a hand edit never
        // depends on the server round trip.
        let mut block = doc
            .get("facts")
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(sent) = answer["facts"].as_object() {
            for (name, value) in sent {
                block.entry(name.clone()).or_insert_with(|| value.clone());
            }
        }
        doc["facts"] = serde_json::Value::Object(block);
        std::fs::write(&path, serde_json::to_string_pretty(&doc)? + "\n")?;
        println!("Filled in {}.", path.display());
    }

    let inferred = answer["answered"].as_array().map_or(0, |list| {
        list.iter()
            .filter(|answer| answer["source"] == "inferred")
            .count()
    });
    let questions = answer["questions"].as_array().cloned().unwrap_or_default();
    println!("The code answered {inferred} facts.");

    // Before anything about what is missing. A value nobody reads is worse
    // than a blank: a blank is reported as a question, and this reads as an
    // answer while switching nothing on. Somebody who wrote `GB` and sees
    // "nothing is left to answer" has no reason to look again.
    let unread_count = answer["unread"].as_array().map_or(0, Vec::len);
    if let Some(unread) = answer["unread"].as_array().filter(|list| !list.is_empty()) {
        println!("\nSome answers match nothing, so they switch no rule on:\n");
        for entry in unread {
            let fact = entry["fact"].as_str().unwrap_or("");
            let value = entry["value"].as_str().unwrap_or("");
            let understood: Vec<&str> = entry["understood"]
                .as_array()
                .map(|list| list.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            println!("  {fact} holds {value:?}, which no rule reads.");
            if !understood.is_empty() {
                println!("    The values that do: {}", understood.join(", "));
            }
        }
    }

    if questions.is_empty() {
        // "Nothing is left to answer" beside a value that reads as an answer
        // and switches nothing on is the contradiction that hid the last one.
        if unread_count > 0 {
            println!("\nEvery fact has an answer. Fix the ones above and run `peko lint`.");
        } else {
            println!("Nothing is left to answer. Run `peko lint`.");
        }
        return Ok(0);
    }

    println!("\n{} facts need an answer from you:\n", questions.len());
    for question in &questions {
        let name = question["fact"].as_str().unwrap_or("");
        let shape = question["shape"].as_str().unwrap_or("string");
        let prompt = question["prompt"].as_str().unwrap_or("");
        let blocks = question["blocks"].as_array().map_or(0, Vec::len);
        println!("  {name} ({shape})");
        println!("    {prompt}");
        println!("    {blocks} rules wait on it.");
    }
    println!("\nEdit the facts block in {}.", config::FILE);
    println!("A fact left null makes every rule that reads it stay silent.");
    Ok(1)
}

/// Read a rejection letter and name the rules behind it.
///
/// The pricing page has sold this since launch, as "rejection diagnosis,
/// unlimited" on the free tier, and the route has answered the whole time.
/// There was no command, so nobody outside this repository could reach it.
///
/// It calls no model and costs nothing, which is why the free tier includes
/// it: somebody who has just been rejected is having a bad day, and putting a
/// paywall in front of the explanation is the wrong moment to ask for money.
pub fn diagnose(root: &Path, file: Option<&Path>, text: Option<&str>) -> Result<i32> {
    let config = Config::load(root)?;
    let key = config.api_key()?;

    let rejection = match (file, text) {
        (Some(path), _) => std::fs::read_to_string(path)
            .with_context(|| format!("could not read {}", path.display()))?,
        (None, Some(inline)) => inline.to_string(),
        (None, None) => {
            // Pasted straight in. A rejection arrives as an email somebody is
            // looking at, and asking them to save it to a file first is a step
            // that loses people.
            use std::io::Read as _;
            let mut buffer = String::new();
            std::io::stdin()
                .read_to_string(&mut buffer)
                .context("nothing was piped in")?;
            buffer
        }
    };
    if rejection.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "there is no rejection text. Pass --file rejection.txt, or pipe the \
             letter in: peko diagnose < rejection.txt"
        ));
    }

    let response = client()?
        .post(format!("{}/diagnose", config.api_url))
        .bearer_auth(&key)
        .json(&serde_json::json!({
            "platform": config.platform,
            "rejection_text": rejection,
        }))
        .send()
        .context("the server did not answer")?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(describe(status, &body));
    }
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    print!("{}", render::diagnosis(&answer));
    Ok(0)
}

/// Report what the store decided.
///
/// The only command that tells us whether a finding was right. It attaches to
/// the last audit run in this project when there was one, because a rejection
/// arrives days later from a terminal somebody closed and asking them to find
/// a job id is asking them not to report.
pub fn report_outcome(
    root: &Path,
    result: &str,
    sections: Option<&str>,
    notes: Option<&str>,
    notes_file: Option<&Path>,
) -> Result<i32> {
    let config = Config::load(root)?;
    let key = config.api_key()?;
    let verdict = outcome::Verdict::parse(result)?;
    let sections = outcome::sections(sections);
    let notes = outcome::notes(notes, notes_file)?;

    if verdict == outcome::Verdict::Rejected && sections.is_empty() && notes.is_none() {
        return Err(anyhow::anyhow!(
            "a rejection needs what the store cited. Pass --sections 3.1.1 or \
             --notes-file rejection.txt, or both. Without either there is \
             nothing to learn from it."
        ));
    }

    let last = outcome::last(root);
    let mut body = serde_json::json!({
        "platform": config.platform,
        "result": verdict.as_str(),
        "sections": sections,
        "flagged": last.as_ref().map(|run| run.flagged.clone()).unwrap_or_default(),
    });
    if let Some(text) = &notes {
        body["notes"] = serde_json::json!(text);
    }
    if let Some(run) = &last {
        body["job_id"] = serde_json::json!(run.job_id);
    }

    let response = client()?
        .post(format!("{}/outcome", config.api_url))
        .bearer_auth(&key)
        .json(&body)
        .send()
        .context("the server did not answer")?;
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(describe(status, &text));
    }

    match verdict {
        outcome::Verdict::Approved => println!("Recorded, and congratulations."),
        outcome::Verdict::Rejected => println!("Recorded. Sorry about the rejection."),
        outcome::Verdict::Withdrawn => println!("Recorded."),
    }
    if last.is_some() {
        println!("Attached to your last audit in this project.");
    } else {
        println!("No audit from this project was on file, so it stands on its own.");
    }
    println!("This is what tells us whether a rule was right. Thank you.");
    Ok(0)
}

pub fn add_override(root: &Path, rule_id: &str, reason: &str) -> Result<i32> {
    let config = Config::load(root)?;
    // Some rules cannot be overridden, and writing one that cannot is worse
    // than refusing: the file says the finding is handled and every later run
    // reports it anyway. Apple rejects an upload holding UIWebView, so no
    // reason a team writes down changes what happens.
    match overridable(&config, rule_id) {
        Ok(true) => {}
        Ok(false) => {
            return Err(anyhow::anyhow!(
                "{rule_id} cannot be overridden. The store rejects the app for it, \
                 so acknowledging it here would change nothing. Fix the finding."
            ));
        }
        // The server did not answer. Write the override rather than block the
        // person, and say the check did not happen.
        Err(error) => eprintln!("peko: could not check whether {rule_id} is overridable: {error}"),
    }

    let path = root.join(config::FILE);
    let mut doc: serde_json::Value = if path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&path)?)?
    } else {
        return Err(anyhow::anyhow!(
            "no {} here. Run `peko init` first.",
            config::FILE
        ));
    };
    let list = doc
        .get_mut("overrides")
        .and_then(|value| value.as_array_mut())
        .ok_or_else(|| anyhow::anyhow!("{} holds no overrides list", config::FILE))?;
    if list
        .iter()
        .any(|entry| entry["rule_id"].as_str() == Some(rule_id))
    {
        println!("{rule_id} is already overridden. Nothing changed.");
        return Ok(0);
    }
    list.push(serde_json::json!({
        "rule_id": rule_id,
        "status": "acknowledged",
        "reason": reason,
    }));
    std::fs::write(&path, serde_json::to_string_pretty(&doc)? + "\n")?;
    println!("{rule_id} is acknowledged. The report still lists it.");
    Ok(0)
}

/// Ask the server whether one rule accepts an override.
pub fn overridable(config: &Config, rule_id: &str) -> Result<bool> {
    let key = config.api_key()?;
    let response = client()?
        .get(format!("{}/rules", config.api_url))
        .bearer_auth(key)
        .query(&[("rule_id", rule_id)])
        .send()
        .context("the server did not answer")?;
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(describe(status, &text));
    }
    let body: serde_json::Value = serde_json::from_str(&text)?;
    let rules = body["rules"].as_array().cloned().unwrap_or_default();
    let found = rules
        .first()
        .ok_or_else(|| anyhow::anyhow!("no rule here has the id {rule_id}"))?;
    Ok(found["overridable"].as_bool().unwrap_or(false))
}

pub fn rules(
    platform: Option<&str>,
    category: Option<&str>,
    show: Option<&str>,
    json: bool,
) -> Result<i32> {
    let config = Config::load(Path::new("."))?;
    let key = config.api_key()?;
    let mut request = client()?
        .get(format!("{}/rules", config.api_url))
        .bearer_auth(key);
    if let Some(value) = platform {
        request = request.query(&[("platform", value)]);
    }
    if let Some(value) = category {
        request = request.query(&[("category", value)]);
    }
    if let Some(value) = show {
        request = request.query(&[("rule_id", value)]);
    }

    let response = request.send().context("the server did not answer")?;
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(describe(status, &text));
    }
    let body: serde_json::Value = serde_json::from_str(&text)?;
    if json || show.is_some() {
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        print!("{}", render::rule_list(&body));
    }
    Ok(0)
}

pub fn status(root: &Path) -> Result<i32> {
    let config = Config::load(root)?;
    println!("Platform:  {}", config.platform);
    println!("API:       {}", config.api_url);
    println!(
        "Key:       {} is {}",
        config.api_key_env,
        if config.api_key().is_ok() {
            "set"
        } else {
            "not set"
        }
    );

    let response = client()?
        .get(format!("{}/health", config.api_url))
        .send()
        .context("the server did not answer")?;
    let body: serde_json::Value = response.json().unwrap_or_default();
    println!(
        "Server:    {}, rules {}, interpretive {}",
        body["status"].as_str().unwrap_or("unknown"),
        body["rule_database_version"].as_str().unwrap_or("unknown"),
        body["interpretive_engine"].as_str().unwrap_or("unknown"),
    );
    Ok(0)
}

/// Say how to supply the key.
///
/// This writes nothing. A key in a file on disk is a key in a backup, in a
/// screen share, and in whatever reads the home directory next.
pub fn login(root: &Path) -> Result<i32> {
    let config = Config::load(root)?;
    println!("Set the key in your environment:");
    println!();
    println!("  export {}=peko_...", config.api_key_env);
    println!();
    println!("Put that in your shell profile, or in the secret store your CI uses.");
    println!("peko writes no key to disk, because a key in a file is a key in a backup.");
    if config.api_key().is_ok() {
        println!();
        println!("{} is set here already.", config.api_key_env);
    }
    Ok(0)
}

#[cfg(test)]
mod limit_tests {
    use super::print_limit;
    use reqwest::StatusCode;

    const BODY: &str = r#"{"error":{"code":"tier_has_no_audits","message":"The free plan runs the mechanical lint, which needs no account and no server. Pro is $20 a month and includes 25 of them. See https://peko.so/pricing."}}"#;

    /// A paywall is not a fault. Printing it as an error next to real ones
    /// sends somebody looking for a break in their project that is not there.
    #[test]
    fn a_payment_required_answer_is_handled_rather_than_raised() {
        assert!(print_limit(StatusCode::PAYMENT_REQUIRED, BODY));
    }

    /// Everything else still raises. A 500 that printed a friendly note and
    /// exited 0 would hide a broken server behind a sales message.
    #[test]
    fn every_other_failure_is_left_to_the_caller() {
        for status in [
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
        ] {
            assert!(
                !print_limit(status, BODY),
                "{status} must still be raised as an error"
            );
        }
    }

    /// A 402 with a body that will not parse still says something useful.
    #[test]
    fn a_limit_with_no_message_still_prints() {
        assert!(print_limit(StatusCode::PAYMENT_REQUIRED, "not json at all"));
    }
}
