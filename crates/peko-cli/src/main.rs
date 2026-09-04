//! `peko`, the client.
//!
//! It gathers files, sends them to the API, and prints what comes back. It
//! holds no rules and no analysis: the rule database is the product and it
//! lives on the server, so a rule fix reaches every caller the moment it
//! promotes rather than when they next upgrade this binary.

use anyhow::Result;
use clap::{Parser, Subcommand};
use peko_cli::{add_override, audit, facts, init, lint, login, rules, status};
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
        /// The most this run may spend, in dollars. Required with `--yes`.
        #[arg(long)]
        max_spend: Option<f64>,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Read what the project answers for itself, and list what is left.
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

fn run() -> Result<i32> {
    match Cli::parse().command {
        Command::Lint {
            path,
            all,
            since,
            platform,
            json,
            fail_on,
            allow_undecided,
        } => lint(
            &path,
            all,
            &since,
            platform.as_deref(),
            json,
            &fail_on,
            allow_undecided,
        ),
        Command::Init { path, platform } => init(&path, platform.as_deref()),
        Command::Facts { path, write } => facts(&path, write),
        Command::Audit {
            path,
            yes,
            max_spend,
            json,
        } => audit(&path, yes, max_spend, json),
        Command::Override {
            rule_id,
            reason,
            path,
        } => add_override(&path, &rule_id, &reason),
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
