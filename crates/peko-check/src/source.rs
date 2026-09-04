//! Source file contents with a line index.

use std::path::{Path, PathBuf};

/// One source file held in memory, with an index that maps a byte offset to a
/// line number.
#[derive(Debug, Clone)]
pub struct SourceFile {
    path: PathBuf,
    relative: PathBuf,
    text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(path: impl Into<PathBuf>, relative: impl Into<PathBuf>, text: String) -> Self {
        let mut line_starts = vec![0usize];
        line_starts.extend(text.match_indices('\n').map(|(index, _)| index + 1));
        Self {
            path: path.into(),
            relative: relative.into(),
            text,
            line_starts,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The path relative to the project root. Reports use this form.
    pub fn relative(&self) -> &Path {
        &self.relative
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// The one-based line number that holds `offset`.
    pub fn line_of(&self, offset: usize) -> usize {
        self.line_starts.partition_point(|start| *start <= offset)
    }

    /// The text of one line, without the line break. Empty when `line` is out
    /// of range.
    pub fn line_text(&self, line: usize) -> &str {
        if line == 0 || line > self.line_starts.len() {
            return "";
        }
        let start = self.line_starts[line - 1];
        let end = self
            .line_starts
            .get(line)
            .map_or(self.text.len(), |next| next - 1);
        self.text[start..end.min(self.text.len())].trim_end_matches('\r')
    }

    /// A snippet centered on `line`, with `context` lines on each side.
    pub fn snippet(&self, line: usize, context: usize) -> String {
        let first = line.saturating_sub(context).max(1);
        let last = (line + context).min(self.line_starts.len());
        (first..=last)
            .map(|number| self.line_text(number))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// True if the file holds one of `needles` anywhere.
    pub fn contains_any(&self, needles: &[String]) -> bool {
        needles
            .iter()
            .any(|needle| self.text.contains(needle.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> SourceFile {
        SourceFile::new(
            "/project/App/View.swift",
            "App/View.swift",
            "import UIKit\n\nlet id = UIDevice.current.identifierForVendor\nprint(id)\n".into(),
        )
    }

    #[test]
    fn maps_offsets_to_lines() {
        let f = file();
        let offset = f.text().find("identifierForVendor").unwrap();
        assert_eq!(f.line_of(offset), 3);
        assert_eq!(f.line_of(0), 1);
    }

    #[test]
    fn reads_one_line() {
        assert_eq!(file().line_text(1), "import UIKit");
        assert_eq!(file().line_text(2), "");
        assert_eq!(file().line_text(99), "");
    }

    #[test]
    fn builds_a_snippet_with_context() {
        let snippet = file().snippet(3, 1);
        assert_eq!(
            snippet,
            "\nlet id = UIDevice.current.identifierForVendor\nprint(id)"
        );
    }

    #[test]
    fn finds_imports() {
        assert!(file().contains_any(&["import UIKit".to_string()]));
        assert!(!file().contains_any(&["import CoreLocation".to_string()]));
    }
}
