//! The guided first run.
//!
//! One command that installs nothing, explains itself, checks two sample
//! apps, and tells the website how it went so the page can move along with
//! the person rather than asking them to click Next at a screen they cannot
//! see the point of.
//!
//! # What is sent
//!
//! The code they pasted, the step reached, the platform, this binary's
//! version, and how many findings there were at each severity. That is all.
//! No path, no file name, no project name, no source. The counts go on the
//! page; the findings stay in the terminal that printed them.
//!
//! # Why not the telemetry channel
//!
//! Because that one is anonymous on purpose and switched off by
//! `DO_NOT_TRACK`. Carrying this on it would break the promise it makes, and
//! would leave everybody who has opted out with a page that never advances.

use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// Where the sample projects come from.
pub const DEMO_URL: &str = "https://peko.so/demo.json";

/// Where to fetch them from, which is the address above unless somebody
/// developing this says otherwise.
///
/// A repository cannot set this, unlike `api_url`, because nothing reads it
/// from a project file. It is an environment variable on the machine of
/// whoever is working on the guide.
fn demo_url() -> String {
    std::env::var("PEKO_DEMO_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEMO_URL.to_string())
}

/// The directory the sample projects are written into.
pub const DEMO_DIR: &str = "peko-demo";

/// How long to wait for the website.
const TIMEOUT: Duration = Duration::from_secs(20);

/// How long to wait when telling the page how it went.
///
/// Short, and a failure is silent. The run has already done its job for the
/// person in front of it, and a page that does not advance is a smaller
/// problem than a command that hangs.
const REPORT_TIMEOUT: Duration = Duration::from_secs(3);

/// The sample projects, as paths and contents.
#[derive(Debug, serde::Deserialize)]
struct Demo {
    files: BTreeMap<String, String>,
}

/// Refuse a path that would write outside the directory we chose.
///
/// This is the whole of the archive problem, handled directly rather than
/// trusted to an extractor. A name may not be absolute, may not climb with
/// `..`, may not carry a drive letter or a root, and may not be empty. What
/// is left can only land under the directory it was given.
fn safe_relative(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.len() > 400 {
        return None;
    }
    // Backslashes are a separator on Windows and an ordinary character
    // elsewhere, which is exactly the gap a crafted name would use.
    if name.contains('\\') || name.contains('\0') {
        return None;
    }
    let path = Path::new(name);
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(piece) => out.push(piece),
            // Every other kind is a way out of the directory.
            Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_)
            | Component::CurDir => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Fetch the sample projects and write them under `into`.
///
/// Returns the two project directories, in the order the guide walks them:
/// the one that fails the lint, then the one that passes it and is still not
/// compliant.
fn fetch_demo(into: &Path) -> Result<(PathBuf, PathBuf)> {
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .context("could not start a download")?;
    let demo: Demo = client
        .get(demo_url())
        .send()
        .context("could not reach peko.so to fetch the sample projects")?
        .error_for_status()
        .context("peko.so did not send the sample projects")?
        .json()
        .context("the sample projects did not arrive in a shape this version reads")?;

    if demo.files.is_empty() {
        anyhow::bail!("peko.so sent no sample projects");
    }

    for (name, contents) in &demo.files {
        let Some(relative) = safe_relative(name) else {
            anyhow::bail!("a sample project named a file this will not write: {name}");
        };
        let path = into.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not make {}", parent.display()))?;
        }
        std::fs::write(&path, contents)
            .with_context(|| format!("could not write {}", path.display()))?;
    }

    let failing = into.join("ios-app");
    let passing = into.join("ios-audit");
    if !failing.join(".pekorc.json").exists() || !passing.join(".pekorc.json").exists() {
        anyhow::bail!("the sample projects arrived incomplete");
    }
    Ok((failing, passing))
}

/// Findings by severity, read off a report.
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct Counts {
    pub errors: i64,
    pub warnings: i64,
    pub infos: i64,
}

impl Counts {
    /// Read the counts a report carries.
    ///
    /// Missing reads as zero rather than as an error. A report shape this
    /// build does not fully know still ran, and the page showing one number
    /// short is better than a guided run that stops.
    #[must_use]
    pub fn of(report: &Value) -> Self {
        let at = |name: &str| {
            report["summary"]["by_severity"][name]
                .as_i64()
                .unwrap_or_default()
        };
        Self {
            errors: at("error"),
            warnings: at("warning"),
            infos: at("info"),
        }
    }

    #[must_use]
    pub fn total(self) -> i64 {
        self.errors + self.warnings + self.infos
    }

    /// One line a person can read.
    #[must_use]
    pub fn sentence(self) -> String {
        if self.total() == 0 {
            return "nothing to report".to_string();
        }
        format!(
            "{} findings: {} error, {} warning, {} info",
            self.total(),
            self.errors,
            self.warnings,
            self.infos
        )
    }
}

/// The findings from one report, trimmed to what a page can show.
///
/// Only ever called for the sample apps. They are ours, they ship in a public
/// repository, and there is nothing in them to protect. A run against
/// somebody's own project sends counts and nothing else: a rule id paired
/// with a file name is a description of code we were never given permission
/// to describe, and it is the sort of thing that is obvious only after it has
/// already been sent.
///
/// Capped, because the page has to render it and this is somebody's network.
fn findings_of(report: &Value) -> Vec<Value> {
    const MOST: usize = 40;
    const LONGEST: usize = 240;

    let trim = |text: &str| -> String {
        if text.chars().count() <= LONGEST {
            return text.to_string();
        }
        text.chars().take(LONGEST).collect::<String>() + "..."
    };

    report["findings"]
        .as_array()
        .map(|all| {
            all.iter()
                .take(MOST)
                .map(|finding| {
                    // The names come from the report, which nests the
                    // place and the remedy rather than flattening them. Read
                    // from the wrong keys this produced findings with no file
                    // and no fix, which looked like the report had neither.
                    serde_json::json!({
                        "rule": finding["rule_id"].as_str().unwrap_or_default(),
                        "severity": finding["severity"].as_str().unwrap_or_default(),
                        "title": trim(finding["title"].as_str().unwrap_or_default()),
                        "detail": trim(finding["message"].as_str().unwrap_or_default()),
                        "file": finding["location"]["file"].as_str().unwrap_or_default(),
                        "line": finding["location"]["line_start"].as_u64(),
                        "fix": trim(
                            finding["remediation"]["summary"]
                                .as_str()
                                .or_else(|| finding["remediation"]["fix"].as_str())
                                .unwrap_or_default(),
                        ),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Wait for the reader before telling the page to move on.
///
/// The work is finished by the time this is called and the page is already
/// showing what it found. Without this it would move on the instant the last
/// report landed, which is the moment somebody has just started reading.
///
/// Returns immediately when nothing is attached to type into it, so a script
/// or a CI job is not left waiting on a key nobody is there to press.
fn wait_for_enter() -> bool {
    use std::io::IsTerminal as _;
    if !std::io::stdin().is_terminal() {
        return true;
    }
    println!();
    println!("Press enter when you have read it, and the page will carry on.");
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    true
}

/// Tell the page how the run went.
///
/// Best effort and silent on failure, the same as the telemetry next door. A
/// person whose network refused this still got their findings, and a guided
/// run that printed a network error at the end would be a worse first
/// impression than a page that needs one click.
fn tell(api_url: &str, code: &str, step: &str, body: Value) {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(REPORT_TIMEOUT)
        .build()
    else {
        return;
    };
    let mut payload = body;
    payload["step"] = Value::String(step.to_string());
    payload["cli_version"] = Value::String(crate::release::running().to_string());
    let _ = client
        .post(format!("{api_url}/onboard/{code}/report"))
        .json(&payload)
        .send();
}

/// Ask for a key that can only run the demo.
///
/// Fetched here rather than shown on the page and pasted, because a key is a
/// thing people should not be in the habit of copying out of a browser, and
/// this one is worth nothing anyway: it belongs to an organisation on the
/// free plan, so it can run the sample app, whose answers are already worked
/// out, and nothing else.
///
/// `None` on any failure. The lint has already run and printed its findings
/// by this point, so a demo that stops short of the audit is a smaller loss
/// than a command that ends in an error about something nobody asked for.
fn demo_key(api_url: &str, code: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Issued {
        key: String,
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;
    // An empty object, not an empty request. A POST carrying neither a body
    // nor a Content-Length is refused by the proxy in front of the server
    // with 411 before it reaches any of our code, which is why this works
    // from curl, which always sends the header, and did not from here.
    let issued: Issued = client
        .post(format!("{api_url}/onboard/{code}/key"))
        .json(&serde_json::json!({}))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .ok()?;
    Some(issued.key)
}

/// Refuse a code that is not one of ours before it goes into a URL.
fn looks_like_a_code(code: &str) -> bool {
    code.len() == 35 && code.starts_with("ob-") && code[3..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Run the guided first run.
///
/// `code` ties this to the page that is waiting. `demo` fetches the sample
/// projects instead of reading the directory somebody is standing in.
pub fn start(root: &Path, code: &str, demo: bool) -> Result<i32> {
    if !looks_like_a_code(code) {
        anyhow::bail!(
            "that does not look like a code from peko.so. \
             Start at https://peko.so/start and copy the command it shows."
        );
    }
    let api_url = crate::config::default_endpoint();

    // Say what this will send before it sends anything. A command that phones
    // home is a reasonable thing to be suspicious of, and the answer to that
    // is to say so first rather than to be quiet about it.
    println!("Peko {} is installed.", crate::release::running());
    println!();
    println!("This checks a project and tells the page you opened how it went:");
    println!("how many findings there were, at which severity, and which platform.");
    println!("No code, no file names, and no project name leave this machine.");
    println!();

    tell(&api_url, code, "installed", serde_json::json!({ "demo": demo }));

    if demo {
        return guided_demo(root, &api_url, code);
    }
    guided_own_project(root, &api_url, code)
}

/// The guided run against the sample projects.
///
/// Two apps, and the contrast between them is the point. The first fails the
/// lint on things a file states. The second passes it and is still not
/// compliant, because everything wrong with it is a thing no file states.
/// That is the argument for the audit tier, and it is much better made by
/// running it than by writing it on a page.
fn guided_demo(root: &Path, api_url: &str, code: &str) -> Result<i32> {
    let into = root.join(DEMO_DIR);
    println!("Fetching two sample apps into {}", into.display());
    let (failing, passing) = fetch_demo(&into)?;
    println!();

    println!("The first one, Northwind, has problems a file states outright.");
    println!();
    let first = lint_quietly(&failing)?;
    print!("{}", crate::render::report(&first));
    let first_counts = Counts::of(&first);
    println!("Northwind: {}", first_counts.sentence());
    println!();

    println!("The second one, Harbor, is the same kind of app written carefully.");
    println!();
    let second = lint_quietly(&passing)?;
    let second_counts = Counts::of(&second);
    println!("Harbor: {}", second_counts.sentence());
    println!();

    tell(
        api_url,
        code,
        "linted",
        serde_json::json!({
            "demo": true,
            "platform": "ios",
            "lint": first_counts,
            "clean": second_counts,
            // Safe to send: these are our own sample projects.
            "findings": {
                "lint": findings_of(&first),
                "clean": findings_of(&second),
            },
        }),
    );

    // The pitch, and it is only worth making because the run above just
    // demonstrated it rather than claimed it.
    println!("Harbor passes the lint. Harbor is not compliant.");
    println!();
    println!("Everything wrong with it is a thing no file states, so no check that");
    println!("reads files can find any of it. That needs a model to read the code,");
    println!("which is the audit. Running one on Harbor now.");
    println!();

    // No account, and no key for anybody to handle. The one fetched here can
    // audit this sample app and nothing else.
    let Some(key) = demo_key(api_url, code) else {
        println!("Could not reach peko.so for the audit. The lint above still stands.");
        println!();
        println!("  {}", peko_page(code));
        return Ok(0);
    };

    match crate::audit_with_key_reporting(&passing, &key) {
        Ok(report) => {
            tell(
                api_url,
                code,
                "audited",
                serde_json::json!({
                    "demo": true,
                    "platform": "ios",
                    "findings": { "audit": findings_of(&report) },
                }),
            );
        }
        Err(error) => {
            println!("The audit did not finish: {error}");
            println!("The lint above still stands.");
        }
    }

    // The page is showing all of it by now. It moves on when the reader says
    // so, not when the work happens to finish.
    println!();
    println!("  {}", peko_page(code));
    wait_for_enter();
    tell(api_url, code, "done", serde_json::json!({ "demo": true }));
    Ok(0)
}

/// The guided run against somebody's own project.
fn guided_own_project(root: &Path, api_url: &str, code: &str) -> Result<i32> {
    if !root.join(crate::config::FILE).exists() {
        println!("Setting this project up first.");
        crate::init(root, None)?;
        println!();
    }
    let report = lint_quietly(root)?;
    print!("{}", crate::render::report(&report));
    let counts = Counts::of(&report);
    println!();
    println!("{}", counts.sentence());

    let platform = crate::config::Config::load(root)
        .map(|config| config.platform)
        .unwrap_or_default();
    tell(
        api_url,
        code,
        "linted",
        serde_json::json!({
            "demo": false,
            "platform": platform,
            "lint": counts,
        }),
    );

    println!();
    println!("Carry on here:");
    println!("  {}", peko_page(code));
    wait_for_enter();
    tell(api_url, code, "done", serde_json::json!({ "demo": false }));
    Ok(0)
}

/// Where the page lives, for a person whose tab has wandered off.
fn peko_page(code: &str) -> String {
    format!("https://peko.so/start?code={code}")
}

/// Lint a directory and hand back the report, printing nothing itself.
fn lint_quietly(root: &Path) -> Result<Value> {
    let config = crate::config::Config::load(root)?;
    crate::local::lint(root, &config.platform, Some(&config.api_url))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name that would climb out of the directory is refused.
    ///
    /// This is the archive problem. It is handled here rather than trusted to
    /// an extractor, because the whole point of fetching files from the
    /// network is that somebody else chose the names.
    #[test]
    fn a_name_that_escapes_the_directory_is_refused() {
        for bad in [
            "../outside",
            "../../etc/passwd",
            "/etc/passwd",
            "ios-app/../../escape",
            "",
            "windows\\path",
            "with\0null",
            "./",
        ] {
            assert!(
                safe_relative(bad).is_none(),
                "{bad:?} would have been written"
            );
        }
    }

    #[test]
    fn an_ordinary_name_is_kept() {
        assert_eq!(
            safe_relative("ios-app/Northwind/Info.plist"),
            Some(PathBuf::from("ios-app/Northwind/Info.plist"))
        );
        assert_eq!(
            safe_relative(".pekorc.json"),
            Some(PathBuf::from(".pekorc.json"))
        );
    }

    /// A path that is only a current directory marker writes nothing.
    #[test]
    fn a_name_that_resolves_to_nothing_is_refused() {
        assert!(safe_relative(".").is_none());
    }

    #[test]
    fn counts_are_read_off_a_report() {
        let report = serde_json::json!({
            "summary": { "by_severity": { "error": 1, "warning": 2, "info": 3 } }
        });
        let counts = Counts::of(&report);
        assert_eq!(counts.total(), 6);
        assert_eq!(counts.sentence(), "6 findings: 1 error, 2 warning, 3 info");
    }

    /// A clean report says so rather than counting to zero three times.
    #[test]
    fn a_clean_report_reads_as_nothing_to_report() {
        let report = serde_json::json!({
            "summary": { "by_severity": { "error": 0, "warning": 0, "info": 0 } }
        });
        assert_eq!(Counts::of(&report).sentence(), "nothing to report");
    }

    /// A shape this build does not know still runs.
    #[test]
    fn a_report_with_no_summary_reads_as_zero_rather_than_failing() {
        assert_eq!(Counts::of(&serde_json::json!({})).total(), 0);
    }

    #[test]
    fn a_code_that_is_not_ours_is_refused() {
        assert!(looks_like_a_code(&format!("ob-{}", "ab".repeat(16))));
        for bad in ["", "ob-", "nope", "ob-zz", "../../etc"] {
            assert!(!looks_like_a_code(bad), "{bad} was accepted");
        }
    }
}
