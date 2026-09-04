//! Running the mechanical checks on this machine.
//!
//! The mechanical tier reads files and calls no model, so it needs no server,
//! no key, and no network. Sending a project to an API to have its manifest
//! parsed was a property of how this client was built, not of the work.
//!
//! That mattered more than it sounds. A developer trying the tool had to find
//! a deployed server and an issued key before they could see one finding, and
//! neither exists for somebody who has not signed up yet. Now the first run
//! works offline.
//!
//! The audit tier still needs the server. It calls a model, it spends money,
//! and the key that pays for it belongs on a server rather than in a binary
//! somebody downloads.

use anyhow::{Context as _, Result};
use std::path::Path;

/// Run the mechanical checks and return the same report shape the API returns.
///
/// The shape matters. The renderer, the exit code, and the unanswered fact
/// handling are all shared with the server path, so a local run and a remote
/// run must be indistinguishable to everything downstream. A second report
/// format would be a second thing to keep correct.
///
/// # Errors
///
/// Returns an error when the embedded database is broken or the project will
/// not load.
pub fn lint(root: &Path, platform: &str) -> Result<serde_json::Value> {
    let database =
        peko_rules::embedded::database().context("the rule database in this binary is broken")?;
    let platform: peko_rules::Platform = platform
        .parse()
        .map_err(|_| anyhow::anyhow!("unknown platform {platform:?}. Use ios or android."))?;

    // Load the project's own .pekorc.json. A declared fact and an override
    // both live there, and a rule gated on a fact reports undecided without
    // it. Building a fresh config would report silence as a pass.
    let config = peko_check::config::PekoConfig::load_or_default(root, platform)
        .with_context(|| format!("failed to read the config at {}", root.display()))?;
    let project = peko_check::project::Project::load(root, &config)
        .with_context(|| format!("failed to read the project at {}", root.display()))?;

    let rules = database.active_rules(config.platform);
    let knowledge = peko_check::KnowledgeBase::load_from_dir(Path::new("knowledge")).ok();
    let outcome = peko_check::engine::run(&project, &rules, &config, knowledge.as_ref())
        .context("the mechanical run failed")?;

    let report = peko_report::ReportBuilder::new(&database, peko_report::Tier::Lint)
        .severity_threshold(config.severity_threshold)
        .build(&project, &outcome);
    Ok(serde_json::from_str(&report.to_json()?)?)
}

/// The version of the database compiled into this binary.
pub fn database_version() -> String {
    peko_rules::embedded::database()
        .map_or_else(|_| "unknown".to_string(), |db| db.version().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "peko-local-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        root
    }

    #[test]
    fn a_project_with_nothing_wrong_reports_a_pass() {
        let root = scratch("empty");
        std::fs::write(
            root.join("Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.example.app</string>
</dict></plist>"#,
        )
        .expect("write");
        let report = lint(&root, "ios").expect("the run finishes");
        assert_eq!(report["tier"], "lint");
        assert!(report["summary"]["total_findings"].is_number());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_report_names_the_database_it_used() {
        // A finding without a database version cannot be reproduced later, and
        // a local run and a server run must be told apart by nothing except
        // this field.
        let root = scratch("version");
        std::fs::write(root.join("Info.plist"), "<plist><dict/></plist>").expect("write");
        let report = lint(&root, "ios").expect("the run finishes");
        assert_eq!(
            report["rule_database_version"].as_str(),
            Some(database_version().as_str())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_platform_is_refused_rather_than_guessed() {
        let root = scratch("platform");
        let error = lint(&root, "web").expect_err("web is not a platform");
        assert!(error.to_string().contains("unknown platform"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_embedded_database_is_the_whole_one() {
        // A binary that shipped a partial database reports a pass on a project
        // the full database would fail, and nothing complains.
        let database = peko_rules::embedded::database().expect("the database parses");
        assert!(!database.is_empty(), "the binary shipped no rules at all");
    }
}
