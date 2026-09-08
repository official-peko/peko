//! Reporting how a run went, unless somebody said not to.
//!
//! # What is sent
//!
//! The command, how long it took, what it exited with, the version, the
//! operating system, and the shape of the project: how many files, which
//! platform, which framework, and how many findings by severity.
//!
//! # What is never sent
//!
//! No source. No file paths. No project name. No bundle id. No API key. None
//! of it is needed to answer the questions this exists for, and each one is a
//! way to identify an app somebody has not shipped yet.
//!
//! # Saying no
//!
//! Three ways, and any one of them is enough:
//!
//! - `PEKO_TELEMETRY=off` in the environment.
//! - `"telemetry": false` in `.pekorc.json`.
//! - `DO_NOT_TRACK=1`, which is the convention other tools honour.
//!
//! The account setting on the website switches it off server side as well, so
//! somebody who turns it off there is not measured even if an old binary
//! keeps sending.
//!
//! # It never gets in the way
//!
//! The send is best effort with a short timeout, and a failure is silent. A
//! lint that found the problem has done its job whatever the network did, and
//! a developer on a plane must not wait on our analytics.

use serde_json::json;
use std::time::Duration;

/// How long to wait for the report. Short on purpose.
const TIMEOUT: Duration = Duration::from_millis(1500);

/// What the environment and the config say.
///
/// Read once and passed in, so the decision itself is a function of its
/// arguments. Reading the environment inside the decision would make it
/// untestable without setting process wide variables, and tests that do that
/// interfere with each other in ways that only show up under load.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Asked {
    /// `DO_NOT_TRACK`, the convention other tools honour.
    pub do_not_track: Option<String>,
    /// `PEKO_TELEMETRY`.
    pub peko_telemetry: Option<String>,
    /// `"telemetry"` in `.pekorc.json`.
    pub config: Option<bool>,
}

impl Asked {
    /// Read the environment and the project's answer.
    #[must_use]
    pub fn read(config: Option<bool>) -> Self {
        Self {
            do_not_track: std::env::var("DO_NOT_TRACK").ok(),
            peko_telemetry: std::env::var("PEKO_TELEMETRY").ok(),
            config,
        }
    }
}

/// Whether this run may be measured.
///
/// Any single refusal wins. A person who set one of these has said no, and
/// weighing them against each other is how a tool ends up ignoring the
/// variable somebody actually set.
#[must_use]
pub fn allowed(asked: &Asked) -> bool {
    // Set to anything but zero. The convention is presence, and honouring
    // only "1" is the usual way to get this wrong.
    if asked
        .do_not_track
        .as_deref()
        .is_some_and(|value| !value.is_empty() && value != "0")
    {
        return false;
    }
    if matches!(
        asked.peko_telemetry.as_deref(),
        Some("off" | "0" | "false" | "no")
    ) {
        return false;
    }
    asked.config != Some(false)
}

/// What one command did.
#[derive(Debug, Clone)]
pub struct Run {
    pub command: &'static str,
    pub milliseconds: u64,
    pub code: i32,
    pub platform: Option<String>,
    pub framework: Option<String>,
    pub files: usize,
    pub errors: u64,
    pub warnings: u64,
    pub infos: u64,
}

impl Run {
    /// The body, with nothing in it that names a project.
    #[must_use]
    pub fn body(&self, anon_id: &str) -> serde_json::Value {
        json!({
            "anon_id": anon_id,
            "client_version": env!("CARGO_PKG_VERSION"),
            "events": [{
                "name": "cli_run",
                "props": {
                    "command": self.command,
                    "ms": self.milliseconds,
                    "code": self.code,
                    "platform": self.platform.clone().unwrap_or_default(),
                    "framework": self.framework.clone().unwrap_or_default(),
                    "files": self.files,
                    "errors": self.errors,
                    "warnings": self.warnings,
                    "infos": self.infos,
                    "os": std::env::consts::OS,
                    "arch": std::env::consts::ARCH,
                    "ci": std::env::var_os("CI").is_some(),
                },
            }],
        })
    }
}

/// A value this machine keeps so two runs join up.
///
/// Written beside the project, next to the last run, so it disappears with a
/// checkout. Not a person, not stable across machines, and nothing anywhere
/// can turn it back into one.
#[must_use]
pub fn anon_id(root: &std::path::Path) -> String {
    let path = root.join(".peko/anon-id");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let fresh = uuid_like();
    if std::fs::create_dir_all(root.join(".peko")).is_ok() {
        let _ = std::fs::write(&path, format!("{fresh}\n"));
    }
    fresh
}

/// A random hex string.
///
/// Not a real UUID and it does not need to be. Nothing joins on it across
/// systems, and pulling in a UUID crate for a value only ever compared to
/// itself is a dependency for nothing.
fn uuid_like() -> String {
    use std::fmt::Write as _;
    use std::hash::{BuildHasher as _, Hasher as _};
    let mut out = String::with_capacity(32);
    for _ in 0..2 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or_default(),
        );
        let _ = write!(out, "{:016x}", hasher.finish());
    }
    out
}

/// Measure a command, and report it if nobody said not to.
///
/// Wraps the work rather than sitting inside it, so a command cannot forget
/// to report and cannot report the wrong duration. The result of the work is
/// handed straight back untouched.
pub fn around<F>(
    root: &std::path::Path,
    command: &'static str,
    endpoint: &str,
    key: Option<&str>,
    config_says: Option<bool>,
    shape: Shape,
    work: F,
) -> anyhow::Result<i32>
where
    F: FnOnce() -> anyhow::Result<i32>,
{
    let started = std::time::Instant::now();
    let outcome = work();
    if !allowed(&Asked::read(config_says)) {
        return outcome;
    }
    let run = Run {
        command,
        milliseconds: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        // A run that ended in an error is the interesting one. Reporting the
        // exit code as zero because the Result was an Err would hide exactly
        // the runs worth looking at.
        code: match &outcome {
            Ok(code) => *code,
            Err(_) => -1,
        },
        platform: shape.platform,
        framework: shape.framework,
        files: shape.files,
        errors: shape.errors,
        warnings: shape.warnings,
        infos: shape.infos,
    };
    report(endpoint, key, &anon_id(root), &run);
    outcome
}

/// What the run was working on. Filled in by the command as it learns it.
#[derive(Debug, Clone, Default)]
pub struct Shape {
    pub platform: Option<String>,
    pub framework: Option<String>,
    pub files: usize,
    pub errors: u64,
    pub warnings: u64,
    pub infos: u64,
}

/// Send it, and never let it matter.
pub fn report(endpoint: &str, key: Option<&str>, anon_id: &str, run: &Run) {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .build()
    else {
        return;
    };
    let mut request = client
        .post(format!("{endpoint}/events"))
        .json(&run.body(anon_id));
    if let Some(key) = key {
        request = request.bearer_auth(key);
    }
    // The result is dropped on purpose. There is nothing a person can do
    // about a failed analytics call, and telling them turns a working run
    // into one that looks broken.
    let _ = request.send();
}

#[cfg(test)]
mod tests {
    use super::{allowed, Asked, Run};

    fn a_run() -> Run {
        Run {
            command: "lint",
            milliseconds: 120,
            code: 0,
            platform: Some("ios".to_string()),
            framework: Some("swiftui".to_string()),
            files: 12,
            errors: 1,
            warnings: 2,
            infos: 3,
        }
    }

    /// Nothing in the body names the project.
    ///
    /// This is the promise the module is built around, and it is the one that
    /// is easy to break later by adding a field that seemed harmless. A path
    /// is the usual one: it carries a developer's name, their employer, and
    /// often the app's.
    #[test]
    fn the_body_names_no_project_and_no_person() {
        let body = a_run().body("anon-1").to_string();
        for forbidden in [
            "/Users",
            "/home",
            "C:\\\\",
            ".swift",
            ".kt",
            "Info.plist",
            "bundle",
            "com.example",
            "@",
        ] {
            assert!(
                !body.contains(forbidden),
                "the report carries {forbidden}: {body}"
            );
        }
    }

    /// Any one refusal is enough. Weighing them against each other is how a
    /// tool ends up ignoring the variable somebody actually set.
    #[test]
    fn any_single_refusal_wins() {
        let refuse = |asked: Asked| assert!(!allowed(&asked), "{asked:?} was measured anyway");

        refuse(Asked {
            do_not_track: Some("1".to_string()),
            ..Asked::default()
        });
        for value in ["off", "0", "false", "no"] {
            refuse(Asked {
                peko_telemetry: Some(value.to_string()),
                ..Asked::default()
            });
        }
        refuse(Asked {
            config: Some(false),
            ..Asked::default()
        });
        // And a refusal anywhere beats a yes everywhere else.
        refuse(Asked {
            do_not_track: Some("1".to_string()),
            peko_telemetry: Some("on".to_string()),
            config: Some(true),
        });
    }

    /// Nobody said no, so the default holds. This is the decision to be
    /// honest about rather than bury.
    #[test]
    fn silence_is_a_yes() {
        assert!(allowed(&Asked::default()));
        assert!(allowed(&Asked {
            config: Some(true),
            ..Asked::default()
        }));
    }

    /// `DO_NOT_TRACK=0` is somebody saying they do not mind, and an empty value
    /// is a shell that expanded nothing rather than a person.
    #[test]
    fn a_zero_in_do_not_track_is_not_a_refusal() {
        assert!(allowed(&Asked {
            do_not_track: Some("0".to_string()),
            ..Asked::default()
        }));
        assert!(allowed(&Asked {
            do_not_track: Some(String::new()),
            ..Asked::default()
        }));
    }
}
