//! Knowing when this binary is out of date, and replacing it.
//!
//! Two separate things, deliberately. The check is quiet, cached, and never
//! in the way. The replacement only happens when somebody asks for it by
//! name, because a tool that rewrites its own binary without being asked is
//! a tool nobody can pin.
//!
//! Note the difference from `update.rs` next door: that keeps the *rule
//! database* current, which happens on its own and needs no permission
//! because it only ever changes what is checked. This is the binary.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// How long between checks.
///
/// A release does not happen hourly, and a network round trip on every lint
/// would be a tax on a command meant to take milliseconds. The answer is
/// cached on disk and re-read until it is a day old.
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// How long to wait for an answer.
///
/// Short, and a failure is silence. Nobody's lint should be slower because we
/// wanted to tell them about a release.
pub const TIMEOUT: Duration = Duration::from_secs(2);

/// What the server says about versions.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Versions {
    pub cli: Cli,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Cli {
    pub latest: String,
    #[serde(default)]
    pub minimum: String,
    #[serde(default)]
    pub update_command: String,
}

/// This binary's own version.
#[must_use]
pub fn running() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn cache_file() -> Option<PathBuf> {
    crate::update::cache_dir().map(|dir| dir.join("latest-version.json"))
}

/// Read the cached answer, if it is recent enough to trust.
fn cached() -> Option<Versions> {
    let path = cache_file()?;
    let age = std::fs::metadata(&path)
        .ok()?
        .modified()
        .ok()
        .and_then(|written| SystemTime::now().duration_since(written).ok())?;
    if age > CHECK_EVERY {
        return None;
    }
    serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()
}

fn remember(versions: &Versions) {
    let Some(path) = cache_file() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string(versions) {
        let _ = std::fs::write(path, text);
    }
}

/// Ask the server what the newest release is.
///
/// Every failure is `None`. No network, a proxy, a server mid deploy, a shape
/// this build cannot read: all of them mean "no idea", and no idea has to be
/// silent rather than a warning about a check nobody asked for.
fn fetch(api_url: &str) -> Option<Versions> {
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .ok()?;
    client
        .get(format!("{api_url}/version"))
        .send()
        .ok()?
        .json::<Versions>()
        .ok()
}

/// What to tell somebody about their version, if anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// Nothing to say.
    Current,
    /// A newer release exists.
    Behind { latest: String, command: String },
    /// This build is older than the server still supports.
    ///
    /// Louder, because this one is not a suggestion. Something is already
    /// broken or about to be, and a quiet note at the end of a long output is
    /// not enough to explain why.
    Unsupported { latest: String, command: String },
}

/// Compare what is running against what the server reports.
///
/// A version that will not parse is treated as current. A developer running a
/// build from source has a version this cannot read, and nagging them about
/// an upgrade to a release they are ahead of would be noise.
#[must_use]
pub fn compare(running: &str, versions: &Versions) -> Notice {
    let Ok(here) = semver::Version::parse(running) else {
        return Notice::Current;
    };
    let Ok(latest) = semver::Version::parse(&versions.cli.latest) else {
        return Notice::Current;
    };
    let command = if versions.cli.update_command.trim().is_empty() {
        "peko update".to_string()
    } else {
        versions.cli.update_command.clone()
    };

    if let Ok(minimum) = semver::Version::parse(&versions.cli.minimum) {
        if here < minimum {
            return Notice::Unsupported {
                latest: versions.cli.latest.clone(),
                command,
            };
        }
    }
    if here < latest {
        return Notice::Behind {
            latest: versions.cli.latest.clone(),
            command,
        };
    }
    Notice::Current
}

/// Whether to say anything at all right now.
///
/// Off when output is being read by something other than a person: a version
/// note in the middle of a machine readable answer is a bug, and a note in CI
/// is a line nobody reads on every build forever.
#[must_use]
pub fn should_check(json: bool) -> bool {
    if json {
        return false;
    }
    if std::env::var("PEKO_NO_UPDATE_CHECK").is_ok_and(|value| !value.trim().is_empty()) {
        return false;
    }
    // The convention every CI sets. A build machine is not a person who can
    // act on the news.
    if std::env::var("CI").is_ok_and(|value| !value.trim().is_empty()) {
        return false;
    }
    true
}

/// The note to print, if there is one.
///
/// Reads the cache first, and only reaches the network once a day. Returns
/// `None` far more often than not, which is the intent.
#[must_use]
pub fn notice(api_url: &str, json: bool) -> Option<Notice> {
    if !should_check(json) {
        return None;
    }
    let versions = match cached() {
        Some(held) => held,
        None => {
            let fetched = fetch(api_url)?;
            remember(&fetched);
            fetched
        }
    };
    match compare(running(), &versions) {
        Notice::Current => None,
        other => Some(other),
    }
}

/// Print a note about the version, if there is one to print.
///
/// To stderr, always. Somebody piping `peko lint --json` into a file has a
/// right to a file that holds only what they asked for, and `should_check`
/// already refuses there, but two guards on the same mistake is the right
/// number when the cost of being wrong is corrupting somebody's output.
pub fn print_notice(notice: &Notice) {
    match notice {
        Notice::Current => {}
        Notice::Behind { latest, command } => {
            eprintln!();
            eprintln!(
                "peko {latest} is out. You have {}. Run {command} to upgrade.",
                running()
            );
        }
        Notice::Unsupported { latest, command } => {
            eprintln!();
            eprintln!(
                "This peko is {}, which the server no longer supports. \
                 Run {command} to get {latest}.",
                running()
            );
        }
    }
}

/// Replace this binary with the newest release.
///
/// This runs the same installer the website serves, for one reason: there is
/// then one piece of code in the world that knows how to put this binary on a
/// machine, and it is the one that has already been tested by everybody who
/// has ever installed it. A second implementation here would be a second
/// thing to get wrong about checksums, paths, and permissions.
pub fn update() -> Result<i32> {
    let current = running();
    println!("peko {current} is installed.");

    // Say what is about to happen before it happens. This downloads and runs
    // a script, and somebody who would rather read it first should be told
    // where it is rather than discover it afterwards.
    println!();
    println!("This runs the installer from https://peko.so/install.sh,");
    println!("which checks the download against its published checksum.");
    println!();

    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg("curl -fsSL https://peko.so/install.sh | sh")
        .status()
        .context("could not run the installer")?;

    if !status.success() {
        anyhow::bail!(
            "the installer did not finish. Install it by hand from \
             https://github.com/official-peko/peko/releases/latest"
        );
    }

    // The version is not reported back, because this process is still the old
    // binary. Saying "updated to 1.9.3" from inside 1.9.2 would be a guess,
    // and the next command they run will say it for real.
    println!();
    println!("Done. Run `peko --version` to see what you have.");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions(latest: &str, minimum: &str) -> Versions {
        Versions {
            cli: Cli {
                latest: latest.to_string(),
                minimum: minimum.to_string(),
                update_command: "peko update".to_string(),
            },
        }
    }

    #[test]
    fn a_current_binary_is_told_nothing() {
        assert_eq!(
            compare("1.9.2", &versions("1.9.2", "1.0.0")),
            Notice::Current
        );
    }

    /// Ahead of the release, which is what a build from source looks like.
    #[test]
    fn a_newer_binary_than_the_release_is_not_nagged() {
        assert_eq!(
            compare("2.0.0", &versions("1.9.2", "1.0.0")),
            Notice::Current,
            "somebody running a build from source was told to downgrade"
        );
    }

    #[test]
    fn a_behind_binary_is_told_what_to_run() {
        match compare("1.8.0", &versions("1.9.2", "1.0.0")) {
            Notice::Behind { latest, command } => {
                assert_eq!(latest, "1.9.2");
                assert_eq!(command, "peko update");
            }
            other => panic!("expected a quiet note, got {other:?}"),
        }
    }

    /// Below the minimum is a different message, because it is not advice.
    #[test]
    fn a_binary_the_server_has_dropped_is_told_more_firmly() {
        assert!(matches!(
            compare("0.9.0", &versions("1.9.2", "1.0.0")),
            Notice::Unsupported { .. }
        ));
    }

    /// A version that will not parse is a build from source. Say nothing.
    #[test]
    fn an_unreadable_version_is_left_alone() {
        assert_eq!(
            compare("dev", &versions("1.9.2", "1.0.0")),
            Notice::Current
        );
        assert_eq!(
            compare("1.9.2", &versions("not-a-version", "")),
            Notice::Current
        );
    }

    /// A server too old to send a minimum still gets the ordinary note.
    #[test]
    fn a_missing_minimum_does_not_read_as_unsupported() {
        match compare("1.8.0", &versions("1.9.2", "")) {
            Notice::Behind { .. } => {}
            other => panic!("a blank minimum made this {other:?}"),
        }
    }

    /// Machine readable output must never carry a note.
    #[test]
    fn json_output_is_never_interrupted_by_a_version_note() {
        assert!(!should_check(true), "a version note would land in the JSON");
    }
}
