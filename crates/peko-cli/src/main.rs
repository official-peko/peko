//! `peko`, the client.
//!
//! It gathers files, sends them to the API, and prints what comes back. It
//! holds no rules and no analysis: the rule database is the product and it
//! lives on the server, so a rule fix reaches every caller the moment it
//! promotes rather than when they next upgrade this binary.

use anyhow::Result;
use clap::{Parser, Subcommand};
use peko_cli::{add_override, audit, facts, init, lint, login, report_outcome, rules, status};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "peko", version, about = "Check an app against store policy")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check the files this commit touched.
    Lint {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Check every file, not only what changed.
        #[arg(long)]
        all: bool,
        /// The commit to compare against.
        #[arg(long, default_value = "HEAD")]
        since: String,
        /// Name the platform, rather than reading it from the config.
        #[arg(long)]
        platform: Option<String>,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
        /// Write the report as SARIF to this file.
        ///
        /// Upload it with github/codeql-action/upload-sarif and every finding
        /// lands on the line it belongs to, in the pull request diff. A report
        /// somebody has to go and read is a report nobody reads.
        #[arg(long)]
        sarif: Option<std::path::PathBuf>,
        /// The severity that makes this command exit non zero.
        #[arg(long, default_value = "error")]
        fail_on: String,
        /// Report a pass even when a fact has no answer.
        ///
        /// A rule that waits on an answer stays silent, so a pass under this
        /// flag covers fewer rules than a pass without it.
        #[arg(long)]
        allow_undecided: bool,
    },
    /// Write a `.pekorc.json` for this project.
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        platform: Option<String>,
    },
    /// Run the interpretive checks. This costs money.
    ///
    /// Without `--yes` it prints the price and stops. That is the default on
    /// purpose: nobody finds out what this costs by being charged for it.
    Audit {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Run it. Without this the command only prints the estimate.
        #[arg(long)]
        yes: bool,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Read what the project answers for itself, and list what is left.
    /// Tell us what the store decided.
    ///
    /// The one thing that says whether a finding was right. It attaches to
    /// the last audit you ran here.
    Outcome {
        /// approved, rejected, or withdrawn.
        result: String,
        /// The project root.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// The guideline sections the store cited, comma separated.
        #[arg(long)]
        sections: Option<String>,
        /// What the store wrote.
        #[arg(long)]
        notes: Option<String>,
        /// A file holding what the store wrote.
        #[arg(long)]
        notes_file: Option<PathBuf>,
    },

    Facts {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Write the answers into the facts block of `.pekorc.json`.
        #[arg(long)]
        write: bool,
    },
    /// Record that a rule does not apply here, with a reason.
    Override {
        rule_id: String,
        /// Why. A reason is required, because an override with none is
        /// indistinguishable later from a mistake.
        #[arg(long)]
        reason: String,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// List the rules the server holds.
    Rules {
        #[arg(long)]
        platform: Option<String>,
        #[arg(long)]
        category: Option<String>,
        /// Show one rule in full.
        #[arg(long)]
        show: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Say where the key is read from, and check the server answers.
    Status {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Explain how to supply the key.
    Login {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("peko: {error:#}");
            std::process::exit(2);
        }
    }
}

/// Run a command, and report how it went unless somebody said not to.
///
/// The wrapper exists so a command cannot forget to report and cannot report
/// the wrong duration. The three ways to say no are read here, once, and the
/// send is best effort with a short timeout: a lint that found the problem
/// has done its job whatever the network did.
fn measured<F>(root: &std::path::Path, command: &'static str, work: F) -> Result<i32>
where
    F: FnOnce(&std::path::Path) -> Result<i32>,
{
    // The config may not parse, or may not exist. Neither is a reason to
    // fail here: the command itself will say so far better than this can.
    let config = peko_cli::config::Config::load(root).ok();
    let endpoint = config
        .as_ref()
        .map_or_else(peko_cli::config::default_endpoint, |c| c.api_url.clone());
    let key = config.as_ref().and_then(|c| c.api_key().ok());
    let says = config
        .as_ref()
        .and_then(peko_cli::config::Config::telemetry);
    let shape = peko_cli::telemetry::Shape {
        platform: config.as_ref().map(|c| c.platform.clone()),
        ..peko_cli::telemetry::Shape::default()
    };
    peko_cli::telemetry::around(
        root,
        command,
        &endpoint,
        key.as_deref(),
        says,
        shape,
        || work(root),
    )
}

/// The project this path sits in, and a word about it when it is not the
/// path itself.
///
/// `peko init` is not routed through here on purpose. It creates a project
/// rather than reading one, and it has its own check for a config above.
fn project(path: &std::path::Path) -> std::path::PathBuf {
    let root = peko_cli::config::project_root(path);
    // Silence is what made the original confusing: a run started in a source
    // directory read a default config, gathered the files under that
    // directory, and reported on a fragment without saying so.
    // Against the canonical form of what was asked for, not the text of it.
    // `peko audit .` in the project root is not a move, and saying so on
    // every ordinary run turns the note into noise nobody reads.
    let asked = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if root != asked {
        if let Some(shown) = root.to_str() {
            eprintln!("peko: reading the project at {shown}");
        }
    }
    root
}

fn run() -> Result<i32> {
    match Cli::parse().command {
        Command::Lint {
            path,
            all,
            since,
            platform,
            json,
            sarif,
            fail_on,
            allow_undecided,
        } => measured(&project(&path), "lint", |root| {
            lint(
                root,
                &peko_cli::LintOptions {
                    all,
                    since: &since,
                    platform: platform.as_deref(),
                    json,
                    sarif: sarif.as_deref(),
                    fail_on: &fail_on,
                    allow_undecided,
                },
            )
        }),
        Command::Init { path, platform } => init(&path, platform.as_deref()),
        Command::Facts { path, write } => {
            measured(&project(&path), "facts", |root| facts(root, write))
        }
        Command::Outcome {
            result,
            path,
            sections,
            notes,
            notes_file,
        } => report_outcome(
            &project(&path),
            &result,
            sections.as_deref(),
            notes.as_deref(),
            notes_file.as_deref(),
        ),
        Command::Audit { path, yes, json } => audit(&project(&path), yes, json),
        Command::Override {
            rule_id,
            reason,
            path,
        } => add_override(&project(&path), &rule_id, &reason),
        Command::Rules {
            platform,
            category,
            show,
            json,
        } => rules(
            platform.as_deref(),
            category.as_deref(),
            show.as_deref(),
            json,
        ),
        Command::Status { path } => status(&path),
        Command::Login { path } => login(&path),
    }
}
