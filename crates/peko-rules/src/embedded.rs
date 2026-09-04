//! The rule database compiled into the binary.
//!
//! A client that needs a server to run a mechanical check needs a deployed
//! server, an issued key, and a network. None of those is available to
//! somebody trying the tool for the first time, and none is needed: the
//! mechanical tier reads files and calls no model.
//!
//! So the database ships inside the binary. The first run works offline, with
//! no account, on an aeroplane. A newer database can replace it later, and
//! this one is the floor that always works.
//!
//! The rules are public policy read into a schema. Embedding them gives away
//! nothing that Apple and Google do not publish. What stays behind is the
//! pipeline that keeps them current and the evidence that says they are right.

use crate::db::{DatabaseManifest, RuleDatabase};
use crate::error::{Result, RuleError};
use crate::schema::Rule;

/// The manifest that shipped with this binary.
pub const MANIFEST_JSON: &str = include_str!("../../../rules/manifest.json");

/// Every rule file, in a stable order.
///
/// The list is written out rather than globbed, because a build script that
/// globs silently ships fewer rules when a file is renamed, and nothing fails.
pub const RULE_FILES: &[(&str, &str)] = &[
    ("AAPL-API", include_str!("../../../rules/AAPL-API.json")),
    ("AAPL-CONTENT", include_str!("../../../rules/AAPL-CONTENT.json")),
    ("AAPL-DATA", include_str!("../../../rules/AAPL-DATA.json")),
    ("AAPL-LEGAL", include_str!("../../../rules/AAPL-LEGAL.json")),
    ("AAPL-META", include_str!("../../../rules/AAPL-META.json")),
    ("AAPL-MINOR", include_str!("../../../rules/AAPL-MINOR.json")),
    ("AAPL-PERM", include_str!("../../../rules/AAPL-PERM.json")),
    ("AAPL-PRIV", include_str!("../../../rules/AAPL-PRIV.json")),
    ("AAPL-SEC", include_str!("../../../rules/AAPL-SEC.json")),
    ("BOTH-DATA", include_str!("../../../rules/BOTH-DATA.json")),
    ("BOTH-LEGAL", include_str!("../../../rules/BOTH-LEGAL.json")),
    ("BOTH-PRIV", include_str!("../../../rules/BOTH-PRIV.json")),
    ("GPLAY-API", include_str!("../../../rules/GPLAY-API.json")),
    ("GPLAY-PERM", include_str!("../../../rules/GPLAY-PERM.json")),
    ("GPLAY-PRIV", include_str!("../../../rules/GPLAY-PRIV.json")),
    ("GPLAY-SEC", include_str!("../../../rules/GPLAY-SEC.json")),
];

/// Build the database that shipped with this binary.
///
/// # Errors
///
/// Returns an error when an embedded file does not parse, which means the
/// binary was built from a broken tree.
pub fn database() -> Result<RuleDatabase> {
    let manifest: DatabaseManifest =
        serde_json::from_str(MANIFEST_JSON).map_err(|source| RuleError::Json {
            path: std::path::PathBuf::from("rules/manifest.json"),
            source,
        })?;
    let mut rules: Vec<Rule> = Vec::new();
    for (name, body) in RULE_FILES {
        let parsed: Vec<Rule> = serde_json::from_str(body).map_err(|source| RuleError::Json {
            path: std::path::PathBuf::from(format!("rules/{name}.json")),
            source,
        })?;
        rules.extend(parsed);
    }
    RuleDatabase::new(manifest, rules)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_database_parses_and_holds_rules() {
        // No count is asserted here. This crate ships in two repositories with
        // different rule sets, and a number baked in here would be wrong in
        // one of them. The directory comparison below is the real check.
        let db = database().expect("the embedded database parses");
        assert!(!db.is_empty(), "the binary shipped no rules at all");
    }

    #[test]
    fn the_embedded_database_matches_the_one_on_disk() {
        // A hand written file list ships fewer rules the moment somebody adds
        // a category and forgets this list. The binary then reports a pass on
        // a project that the full database would fail, and nothing complains.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("the repository root")
            .join("rules");
        let on_disk = RuleDatabase::load_from_dir(&root).expect("the directory loads");
        let embedded = database().expect("the embedded database parses");
        assert_eq!(
            embedded.len(),
            on_disk.len(),
            "the embedded list is missing a rule file"
        );
        assert_eq!(embedded.version(), on_disk.version());
    }

    #[test]
    fn every_embedded_rule_file_holds_rules() {
        // An empty file reads as valid JSON and ships nothing.
        for (name, body) in RULE_FILES {
            let parsed: Vec<Rule> =
                serde_json::from_str(body).unwrap_or_else(|_| panic!("{name} parses"));
            assert!(!parsed.is_empty(), "{name} shipped no rules");
        }
    }
}
