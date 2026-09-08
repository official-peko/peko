//! Colour, when the terminal is one and the reader wants it.
//!
//! Three things switch it off, and all three are somebody telling us
//! something rather than us guessing:
//!
//! - `NO_COLOR` is set, whatever its value. The convention is presence, not
//!   truthiness, and honouring only "1" is the usual way to get it wrong.
//! - `TERM` is `dumb`, which is a terminal saying it cannot render this.
//! - Output is not a terminal at all. Escape codes in a file a script greps
//!   turn a working pipeline into a puzzle, and this output is read by tools
//!   as often as by people.
//!
//! `CLICOLOR_FORCE` overrides the last of those, for somebody piping into a
//! pager that does understand colour.
//!
//! Colour carries no information on its own here. Every severity is spelled
//! out in words beside it, because a reader who cannot see the difference
//! between red and amber still has to be able to read the report.

use std::io::IsTerminal;

/// Whether to write escape codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    on: bool,
}

impl Style {
    /// What this terminal and this environment ask for.
    #[must_use]
    pub fn detect() -> Self {
        Self {
            on: wanted(asked(), goes()),
        }
    }

    /// Colour on, whatever the terminal is. For a test.
    #[must_use]
    pub const fn forced() -> Self {
        Self { on: true }
    }

    /// Colour off, for a test or a caller that wants plain text.
    #[must_use]
    pub const fn plain() -> Self {
        Self { on: false }
    }

    /// Wrap `text` in a code, or hand it back untouched.
    #[must_use]
    pub fn paint(self, code: &str, text: &str) -> String {
        if self.on && !text.is_empty() {
            format!("\u{1b}[{code}m{text}\u{1b}[0m")
        } else {
            text.to_string()
        }
    }

    /// A severity, in the colour the site uses for it.
    #[must_use]
    pub fn severity(self, severity: &str) -> String {
        let code = match severity {
            "error" => "1;38;5;166",   // the blocking orange
            "warning" => "1;38;5;136", // the amber that means read this
            _ => "1;38;5;66",          // quieter, for something to know
        };
        self.paint(code, severity)
    }

    /// A count beside its severity, coloured only when it is not zero.
    ///
    /// A zero in the colour that means "stop" reads as a problem for a second
    /// before the number registers. Zero is the good news.
    #[must_use]
    pub fn severity_count(self, severity: &str, count: u64) -> String {
        if count == 0 {
            self.dim(&count.to_string())
        } else {
            self.severity_number(severity, count)
        }
    }

    /// The number, in the colour of its severity.
    fn severity_number(self, severity: &str, count: u64) -> String {
        let code = match severity {
            "error" => "1;38;5;166",
            "warning" => "1;38;5;136",
            _ => "1;38;5;66",
        };
        self.paint(code, &count.to_string())
    }

    /// A rule id. Dim, because it is a reference rather than the point.
    #[must_use]
    pub fn dim(self, text: &str) -> String {
        self.paint("2", text)
    }

    /// A heading, or the part of a line that says what happened.
    #[must_use]
    pub fn bold(self, text: &str) -> String {
        self.paint("1", text)
    }

    /// A file and line. The part somebody is about to go and open.
    #[must_use]
    pub fn place(self, text: &str) -> String {
        self.paint("4;36", text)
    }

    /// What to do about it.
    #[must_use]
    pub fn fix(self, text: &str) -> String {
        self.paint("32", text)
    }
}

/// Read what the environment says.
fn asked() -> Asked {
    if std::env::var_os("NO_COLOR").is_some()
        || std::env::var("TERM").ok().as_deref() == Some("dumb")
    {
        Asked::None
    } else if std::env::var_os("CLICOLOR_FORCE").is_some() {
        Asked::Forced
    } else {
        Asked::Unsaid
    }
}

/// Where this run is writing to.
fn goes() -> Goes {
    if std::io::stdout().is_terminal() {
        Goes::ToTerminal
    } else {
        Goes::Elsewhere
    }
}

/// What the reader asked for.
///
/// Four flags collapse into this. `NO_COLOR` and a dumb terminal both mean
/// refused, and they mean it for the same reason: somebody said so. Reading
/// them as two separate bools invited a caller to weigh one against the other,
/// and neither is weighed against anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    /// `NO_COLOR`, or a terminal saying it cannot render this.
    None,
    /// `CLICOLOR_FORCE`, for a pipe into something that does understand it.
    Forced,
    /// Nobody said, so the destination decides.
    Unsaid,
}

/// Where the output goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Goes {
    /// To a terminal, where colour is read by a person.
    ToTerminal,
    /// To a pipe or a file, where it is read by a program.
    Elsewhere,
}

/// The decision, apart from the environment, so a test can drive it.
#[must_use]
fn wanted(asked: Asked, goes: Goes) -> bool {
    match asked {
        // Refused wins over everything, including a force. A person who set
        // NO_COLOR asked for no colour, and a library overriding that is the
        // reason the variable exists.
        Asked::None => false,
        Asked::Forced => true,
        Asked::Unsaid => goes == Goes::ToTerminal,
    }
}

#[cfg(test)]
mod tests {
    use super::{wanted, Asked, Goes, Style};

    #[test]
    fn a_terminal_gets_colour_and_a_pipe_does_not() {
        assert!(wanted(Asked::Unsaid, Goes::ToTerminal));
        assert!(
            !wanted(Asked::Unsaid, Goes::Elsewhere),
            "escape codes in a piped report turn a grep into a puzzle"
        );
    }

    /// Set at all, whatever the value. Honouring only "1" is the usual way to
    /// get this wrong, and the convention is presence.
    #[test]
    fn a_refusal_wins_over_everything() {
        assert!(!wanted(Asked::None, Goes::ToTerminal));
        assert!(!wanted(Asked::None, Goes::Elsewhere));
    }

    /// For somebody piping into a pager that understands colour.
    #[test]
    fn a_force_reaches_past_a_pipe() {
        assert!(wanted(Asked::Forced, Goes::Elsewhere));
    }

    #[test]
    fn plain_writes_no_escape_codes() {
        let style = Style::plain();
        assert_eq!(style.severity("error"), "error");
        assert_eq!(style.place("App/View.swift:24"), "App/View.swift:24");
        assert!(!style.bold("x").contains('\u{1b}'));
    }

    /// Zero is the good news, so it is not painted the colour that means stop.
    #[test]
    fn a_zero_count_is_not_alarming() {
        let style = Style::forced();
        assert!(!style.severity_count("error", 0).contains("38;5;166"));
        assert!(style.severity_count("error", 1).contains("38;5;166"));
    }
}
