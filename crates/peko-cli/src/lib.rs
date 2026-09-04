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
pub mod render;

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

pub fn lint(
    root: &Path,
    all: bool,
    since: &str,
    platform: Option<&str>,
    json: bool,
    fail_on: &str,
    allow_undecided: bool,
) -> Result<i32> {
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
        return lint_locally(root, &config, all, since, json, fail_on, allow_undecided);
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

/// Run the mechanical checks here, with the database compiled in.
fn lint_locally(
    root: &Path,
    config: &Config,
    all: bool,
    since: &str,
    json: bool,
    fail_on: &str,
    allow_undecided: bool,
) -> Result<i32> {
    if !all {
        // The server path sends only what changed. The local path reads the
        // whole project, because the engine walks the tree itself and a
        // partial read would report a pass on a file it never opened.
        let changed = gather::changed_files(root, since);
        if changed.is_empty() {
            println!("Nothing changed since {since}. Checking the whole project.");
        }
    }
    let report = local::lint(root, &config.platform)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(render::exit_code(&report, fail_on));
    }
    print!("{}", render::report(&report));
    println!(
        "\nChecked on this machine with rule database {}. The audit tier needs \
         a key: run `peko login`.",
        local::database_version()
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
        println!("{} is already here. Nothing changed.", path.display());
        println!("Run `peko facts` to fill in what is missing.");
        return Ok(0);
    }
    let platform = match platform {
        Some(named) => named.to_string(),
        None => config::detect_platform(root)?,
    };
    let doc = serde_json::json!({
        "version": 1,
        "platform": platform,
        "api_key_env": "PEKO_API_KEY",
        "facts": {},
        "overrides": [],
    });
    std::fs::write(&path, serde_json::to_string_pretty(&doc)? + "\n")?;
    println!("Wrote {}.", path.display());

    // A file with an empty facts block is not usable yet. Every rule that
    // needs an answer reports undecided, and undecided reads like a pass. So
    // fill in what the project answers for itself, and name the rest.
    match facts(root, true) {
        Ok(code) => Ok(code),
        Err(error) => {
            eprintln!("peko: could not reach the server to fill in the facts: {error}");
            println!("Run `peko facts --write` when the server answers.");
            Ok(0)
        }
    }
}

/// Run the audit, or say what it would cost.
///
/// The estimate always runs first and it always costs nothing. `--yes` is the
/// only thing that spends money, and it needs a number with it.
pub fn audit(root: &Path, yes: bool, max_spend: Option<f64>, json: bool) -> Result<i32> {
    let config = Config::load(root)?;
    let key = config.api_key()?;
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
            return Err(describe(status, &text));
        }
        serde_json::from_str(&text)?
    };

    let cost = estimate["estimated_cost_usd"].as_f64().unwrap_or(0.0);
    let blockers = estimate["blockers"].as_array().cloned().unwrap_or_default();

    if !yes {
        print!("{}", render::estimate(&estimate));
        println!();
        if blockers.is_empty() {
            println!("Nothing is stopping this run.");
            println!(
                "To run it:  peko audit --yes --max-spend {:.2}",
                cost.max(0.01)
            );
        } else {
            println!("Fix the above first, then run it with --yes --max-spend N.");
        }
        // Printing a price is not a failure, and a blocker is.
        return Ok(i32::from(!blockers.is_empty()));
    }

    let Some(limit) = max_spend else {
        return Err(anyhow::anyhow!(
            "--yes needs --max-spend N. The estimate is ${cost:.2}. \
             A run without a cap is a run with no answer to how much it cost."
        ));
    };
    if limit < cost {
        return Err(anyhow::anyhow!(
            "The estimate is ${cost:.2} and --max-spend is ${limit:.2}. \
             Raise the limit or narrow the project."
        ));
    }

    let body = serde_json::json!({
        "platform": config.platform,
        "files": files,
        "overrides": overrides,
        "confirm": true,
        "max_spend_usd": limit,
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

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(render::exit_code(&report["report"], "error"));
    }
    print!("{}", render::report(&report["report"]));
    println!();
    println!("Spent ${:.2}.", report["spent_usd"].as_f64().unwrap_or(0.0));
    Ok(render::exit_code(&report["report"], "error"))
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

    if questions.is_empty() {
        println!("Nothing is left to answer. Run `peko lint`.");
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
