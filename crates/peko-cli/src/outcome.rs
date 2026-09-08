//! Telling us what the store decided.
//!
//! Everything else in this tool measures what Peko did. This is the one thing
//! that says whether what Peko did was right, and it can only come from the
//! person who got the letter.
//!
//! # Remembering the audit
//!
//! A rejection arrives days after the run, from a terminal somebody closed.
//! Asking them to find a job id is asking them not to report, so the last
//! audit is written to `.peko/last-run.json` and read back here.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Where the last audit is remembered.
const STATE: &str = ".peko/last-run.json";

/// What the last audit was.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastRun {
    pub job_id: String,
    pub finished_on: String,
    /// The rules it raised, so a report can say what we predicted without
    /// the job, which the server deletes an hour after it finishes.
    #[serde(default)]
    pub flagged: Vec<String>,
}

/// Write down the audit that just finished.
///
/// Best effort. A report that cannot be written must not fail the audit that
/// produced it.
pub fn remember(root: &Path, run: &LastRun) {
    let dir = root.join(".peko");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(text) = serde_json::to_string_pretty(run) {
        let _ = std::fs::write(root.join(STATE), text + "\n");
    }
    // A state directory in somebody's repository is theirs to ignore, and
    // finding it in a commit is a small betrayal. Adding the line is not, so
    // it is added once and never repeated.
    ignore(root);
}

/// Add `.peko/` to `.gitignore`, once.
fn ignore(root: &Path) {
    let path = root.join(".gitignore");
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if current.lines().any(|line| line.trim() == ".peko/") {
        return;
    }
    let mut next = current;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(".peko/\n");
    let _ = std::fs::write(path, next);
}

/// Read what the last audit was, if there was one.
#[must_use]
pub fn last(root: &Path) -> Option<LastRun> {
    let text = std::fs::read_to_string(root.join(STATE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// What a person is reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Approved,
    Rejected,
    Withdrawn,
}

impl Verdict {
    /// Read one from what somebody typed.
    ///
    /// Generous about the wording, because "rejected" and "reject" and
    /// "rejection" all mean the same thing to the person typing them and a
    /// refusal over which one they picked is a report we do not get.
    pub fn parse(text: &str) -> Result<Self> {
        let lowered = text.trim().to_lowercase();
        match lowered.as_str() {
            "approved" | "approve" | "accepted" | "accept" | "pass" | "passed" => {
                Ok(Self::Approved)
            }
            "rejected" | "reject" | "rejection" | "refused" | "fail" | "failed" => {
                Ok(Self::Rejected)
            }
            "withdrawn" | "withdraw" | "cancelled" | "canceled" => Ok(Self::Withdrawn),
            other => Err(anyhow::anyhow!(
                "{other:?} is not a result. Use approved, rejected, or withdrawn."
            )),
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Withdrawn => "withdrawn",
        }
    }
}

/// Guideline sections out of a comma or space separated list.
///
/// Apple writes 2.5.1 and Google writes a policy name, so nothing here
/// validates the shape. It splits, trims, and drops the empties.
#[must_use]
pub fn sections(raw: Option<&str>) -> Vec<String> {
    raw.map(|text| {
        text.split([',', ';', ' ', '\n'])
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ToString::to_string)
            .collect()
    })
    .unwrap_or_default()
}

/// Read the letter, from a file or from what was typed.
pub fn notes(inline: Option<&str>, file: Option<&Path>) -> Result<Option<String>> {
    if let Some(path) = file {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("could not read {}", path.display()))?;
        return Ok(Some(text));
    }
    Ok(inline.map(ToString::to_string))
}

#[cfg(test)]
mod tests {
    use super::{last, remember, sections, LastRun, Verdict};

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "peko-outcome-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        path
    }

    /// A rejection arrives days later from a terminal somebody closed.
    #[test]
    fn the_last_audit_survives_the_terminal() {
        let root = scratch("remember");
        assert!(last(&root).is_none());
        remember(
            &root,
            &LastRun {
                job_id: "job-1".to_string(),
                finished_on: "2026-09-08".to_string(),
                flagged: vec!["AAPL-PAY-012".to_string()],
            },
        );
        let found = last(&root).expect("the run is remembered");
        assert_eq!(found.job_id, "job-1");
        assert_eq!(found.flagged, vec!["AAPL-PAY-012"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A state directory in somebody's repository is theirs to ignore, and
    /// finding it in a commit is a small betrayal.
    #[test]
    fn the_state_directory_is_ignored_once() {
        let root = scratch("ignore");
        std::fs::write(root.join(".gitignore"), "target\n").expect("write");
        for _ in 0..3 {
            remember(
                &root,
                &LastRun {
                    job_id: "job-2".to_string(),
                    finished_on: "2026-09-08".to_string(),
                    flagged: vec![],
                },
            );
        }
        let text = std::fs::read_to_string(root.join(".gitignore")).expect("read");
        assert_eq!(
            text.matches(".peko/").count(),
            1,
            "the ignore line was added more than once: {text:?}"
        );
        assert!(text.contains("target"), "the existing file was overwritten");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A gitignore with no trailing newline must not have the line glued to
    /// the last entry.
    #[test]
    fn an_ignore_without_a_final_newline_is_not_corrupted() {
        let root = scratch("newline");
        std::fs::write(root.join(".gitignore"), "target").expect("write");
        remember(
            &root,
            &LastRun {
                job_id: "job-3".to_string(),
                finished_on: "2026-09-08".to_string(),
                flagged: vec![],
            },
        );
        let text = std::fs::read_to_string(root.join(".gitignore")).expect("read");
        assert!(text.contains("target\n.peko/"), "{text:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A refusal over which synonym somebody picked is a report we do not get.
    #[test]
    fn the_result_is_read_generously() {
        for text in ["rejected", "Reject", " rejection ", "FAILED"] {
            assert_eq!(Verdict::parse(text).expect(text), Verdict::Rejected);
        }
        for text in ["approved", "accept", "passed"] {
            assert_eq!(Verdict::parse(text).expect(text), Verdict::Approved);
        }
        assert!(Verdict::parse("maybe").is_err());
    }

    #[test]
    fn sections_split_on_whatever_somebody_typed() {
        assert_eq!(
            sections(Some("3.1.1, 5.1.1;2.1  4.2")),
            vec!["3.1.1", "5.1.1", "2.1", "4.2"]
        );
        assert!(sections(None).is_empty());
        assert!(sections(Some("  ,, ")).is_empty());
    }
}
