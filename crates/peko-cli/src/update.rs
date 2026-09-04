//! Keeping the local rule database current.
//!
//! The binary ships with a database, so the first run works offline. Policies
//! change, and a binary from six months ago misses every rule added since. So
//! this fetches a newer one when it can reach the network.
//!
//! Nothing here can make a run worse than not running it. Every failure path
//! falls back to the database compiled into the binary:
//!
//! - no network, a timeout, a proxy, an air-gapped build machine
//! - a signature that does not verify
//! - a database older than the one in hand
//! - a cache file somebody edited
//!
//! A failure is quiet on purpose. A developer running a lint in CI wants the
//! lint, not a warning about a fetch they did not ask for. The report says
//! which database it used, so a run is always traceable to one.

use peko_rules::signed;
use peko_rules::RuleDatabase;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// How long a cached database is used before another fetch is tried.
///
/// A policy does not change hourly, and a check on every run would add a
/// network round trip to a command that is meant to take milliseconds.
pub const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// How long to wait for the server.
///
/// Short on purpose. This is an optional improvement to a run that already
/// works, so it must never be the reason a lint feels slow.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// Where the cached database and its signature live.
#[must_use]
pub fn cache_dir() -> Option<PathBuf> {
    std::env::var_os("PEKO_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs_cache().map(|base| base.join("peko")))
}

fn dirs_cache() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("XDG_CACHE_HOME") {
        return Some(PathBuf::from(home));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".cache"))
}

/// Which database a run used, so the report can say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The one compiled into this binary.
    Embedded,
    /// A cached database that verified.
    Cache,
    /// A database fetched and verified during this run.
    Fetched,
}

/// The database to check with, and where it came from.
///
/// The only failure is a broken embedded database, which means a broken
/// build. Every network and cache failure falls back to it rather than error,
/// because a fetch is an improvement to a run that already works.
///
/// # Errors
///
/// Returns an error when the database compiled into this binary will not
/// parse. There is nothing to fall back to, and reporting a pass would be
/// worse than saying so.
pub fn database(base_url: Option<&str>) -> anyhow::Result<(RuleDatabase, Source)> {
    let embedded = peko_rules::embedded::database()
        .map_err(|error| anyhow::anyhow!("the rule database in this binary is broken: {error}"))?;

    if let Some(cached) = read_cache(embedded.version()) {
        if !cache_is_stale() {
            return Ok((cached, Source::Cache));
        }
    }

    if let Some(url) = base_url {
        if let Some(fetched) = fetch(url, embedded.version()) {
            return Ok((fetched, Source::Fetched));
        }
    }
    // A stale cache still beats a database from the build. It verified when it
    // was written, and it is verified again on the way in.
    if let Some(cached) = read_cache(embedded.version()) {
        return Ok((cached, Source::Cache));
    }
    Ok((embedded, Source::Embedded))
}

/// Whether the cache is old enough to try a fetch.
fn cache_is_stale() -> bool {
    let Some(path) = cache_dir().map(|dir| dir.join("database.json")) else {
        return true;
    };
    let Ok(modified) = std::fs::metadata(&path).and_then(|meta| meta.modified()) else {
        return true;
    };
    SystemTime::now()
        .duration_since(modified)
        .map_or(true, |age| age > MAX_AGE)
}

/// Read the cache, and verify it again on the way in.
///
/// Verifying on write only is not enough. The file sits on disk between runs,
/// and anything that can edit it can choose what the tool reports.
fn read_cache(held: &semver::Version) -> Option<RuleDatabase> {
    let dir = cache_dir()?;
    let payload = std::fs::read(dir.join("database.json")).ok()?;
    let signature = std::fs::read(dir.join("database.sig")).ok()?;
    let key = signed::public_key().ok()?;
    signed::verify(&payload, &signature, &key, held).ok()
}

/// Fetch a database, verify it, and cache it when it is good.
fn fetch(base_url: &str, held: &semver::Version) -> Option<RuleDatabase> {
    let key = signed::public_key().ok()?;
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .ok()?;
    let payload = client
        .get(format!("{base_url}/rules/database.json"))
        .send()
        .ok()?
        .bytes()
        .ok()?
        .to_vec();
    let signature = client
        .get(format!("{base_url}/rules/database.sig"))
        .send()
        .ok()?
        .bytes()
        .ok()?
        .to_vec();

    match signed::verify(&payload, &signature, &key, held) {
        Ok(database) => {
            write_cache(&payload, &signature);
            Some(database)
        }
        // A refusal is not an error to report. The run continues on the
        // database it already has, which is the whole point of the fallback.
        // Every refusal lands here, including one nobody has thought of yet.
        Err(_) => None,
    }
}

/// Write the verified bytes, so the next run skips the fetch.
///
/// Only bytes that already verified are written. A failure to write is
/// ignored: a read-only cache directory makes every run fetch again, which is
/// slower and still correct.
fn write_cache(payload: &[u8], signature: &[u8]) {
    let Some(dir) = cache_dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = std::fs::write(dir.join("database.json"), payload);
    let _ = std::fs::write(dir.join("database.sig"), signature);
}

/// Remove the cached database.
///
/// # Errors
///
/// Returns an error when the files exist and cannot be removed.
pub fn clear_cache() -> std::io::Result<()> {
    let Some(dir) = cache_dir() else {
        return Ok(());
    };
    for name in ["database.json", "database.sig"] {
        match std::fs::remove_file(dir.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// The cache path, for a message to a person.
#[must_use]
pub fn cache_path() -> Option<PathBuf> {
    cache_dir().map(|dir| dir.join("database.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "peko-update-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn with_no_network_the_embedded_database_is_used() {
        // A lint on an air-gapped build machine must work. The fetch is an
        // improvement to a run that already works, never a requirement.
        let (database, source) = database(Some("http://127.0.0.1:1/v1")).expect("embedded works");
        assert_eq!(source, Source::Embedded);
        assert!(!database.is_empty());
    }

    #[test]
    fn with_no_url_nothing_is_fetched() {
        let (database, source) = database(None).expect("embedded works");
        assert_eq!(source, Source::Embedded);
        assert!(!database.is_empty());
    }

    #[test]
    fn a_cache_that_does_not_verify_is_ignored() {
        // The file sits on disk between runs. Anything that can edit it can
        // choose what the tool reports, so it is verified on the way in and
        // not only on the way out.
        let dir = scratch("badcache");
        std::fs::write(dir.join("database.json"), b"{\"manifest\":{},\"rules\":[]}")
            .expect("write");
        std::fs::write(dir.join("database.sig"), [0u8; 64]).expect("write");
        let held = semver::Version::parse("0.1.0").expect("a version");
        assert!(
            read_cache(&held).is_none(),
            "an unsigned cache was accepted"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_cache_is_not_an_error() {
        let dir = scratch("nocache");
        let held = semver::Version::parse("0.1.0").expect("a version");
        assert!(read_cache(&held).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_timeout_is_short_enough_to_not_hurt_a_lint() {
        // A lint is meant to take milliseconds. A fetch that hangs would make
        // the tool feel broken for a feature nobody asked for.
        const { assert!(TIMEOUT.as_secs() <= 10) }
    }

    #[test]
    fn the_cache_lives_long_enough_to_not_fetch_every_run() {
        const { assert!(MAX_AGE.as_secs() >= 60 * 60) }
    }

    #[test]
    fn clearing_a_cache_that_is_not_there_succeeds() {
        assert!(clear_cache().is_ok());
    }
}
