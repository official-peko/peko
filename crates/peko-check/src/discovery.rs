//! Project file discovery.
//!
//! The walk honors `.gitignore`, a built-in list of directories that hold
//! build output or vendored code, and the `exclude_paths` globs from
//! `.pekorc.json`.

use crate::error::{CheckError, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use peko_parse::{classify, FileKind};
use std::path::{Path, PathBuf};

/// Directories that never hold first-party project files.
pub const ALWAYS_EXCLUDED_DIRS: &[&str] = &[
    ".git",
    ".build",
    "build",
    "DerivedData",
    "Pods",
    "Carthage",
    "node_modules",
    ".gradle",
    "vendor",
    "target",
];

/// The largest file the checker reads. A larger file is skipped and recorded.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// One discovered file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    pub path: PathBuf,
    /// The path relative to the project root.
    pub relative: PathBuf,
    pub kind: FileKind,
    pub size_bytes: u64,
}

/// The result of a walk.
#[derive(Debug, Clone, Default)]
pub struct Discovery {
    pub files: Vec<DiscoveredFile>,
    /// Files skipped because they exceed [`MAX_FILE_BYTES`].
    pub skipped_large: Vec<PathBuf>,
}

/// Build a glob set from user patterns.
///
/// A pattern that ends with `/` matches a directory and everything under it,
/// so `Tests/` behaves the way a developer expects.
pub fn build_glob_set(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        for expanded in expand_pattern(pattern) {
            let glob = Glob::new(&expanded).map_err(|source| CheckError::Glob {
                pattern: pattern.clone(),
                source,
            })?;
            builder.add(glob);
        }
    }
    builder.build().map_err(|source| CheckError::Glob {
        pattern: patterns.join(", "),
        source,
    })
}

fn expand_pattern(pattern: &str) -> Vec<String> {
    let trimmed = pattern.trim_start_matches("./");
    if trimmed.ends_with('/') {
        let base = trimmed.trim_end_matches('/');
        return vec![
            base.to_string(),
            format!("{base}/**"),
            format!("**/{base}/**"),
        ];
    }
    if trimmed.contains('*') {
        return vec![trimmed.to_string()];
    }
    // A bare path may name a file or a directory. Cover both.
    vec![trimmed.to_string(), format!("{trimmed}/**")]
}

/// Walk a project directory and classify every file.
pub fn discover(root: &Path, exclude_paths: &[String]) -> Result<Discovery> {
    if !root.is_dir() {
        return Err(CheckError::NotADirectory {
            path: root.to_path_buf(),
        });
    }
    let excludes = build_glob_set(exclude_paths)?;
    let mut discovery = Discovery::default();

    let walk = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .parents(false)
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !ALWAYS_EXCLUDED_DIRS.contains(&name.as_ref())
        })
        .build();

    for entry in walk {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path();
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        if excludes.is_match(relative) {
            continue;
        }
        let Some(kind) = classify(path) else { continue };
        let size_bytes = entry.metadata().map(|meta| meta.len()).unwrap_or_default();
        if size_bytes > MAX_FILE_BYTES {
            discovery.skipped_large.push(relative.to_path_buf());
            continue;
        }
        discovery.files.push(DiscoveredFile {
            path: path.to_path_buf(),
            relative: relative.to_path_buf(),
            kind,
            size_bytes,
        });
    }

    discovery.files.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(discovery)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_patterns_cover_their_contents() {
        let set = build_glob_set(&["Tests/".into()]).unwrap();
        assert!(set.is_match(Path::new("Tests/AppTests.swift")));
        assert!(set.is_match(Path::new("App/Tests/AppTests.swift")));
        assert!(!set.is_match(Path::new("App/View.swift")));
    }

    #[test]
    fn star_patterns_pass_through() {
        let set = build_glob_set(&["**/*.generated.swift".into()]).unwrap();
        assert!(set.is_match(Path::new("App/Model.generated.swift")));
        assert!(!set.is_match(Path::new("App/Model.swift")));
    }

    #[test]
    fn bare_paths_match_a_file_or_a_directory() {
        let set = build_glob_set(&["Fixtures".into()]).unwrap();
        assert!(set.is_match(Path::new("Fixtures")));
        assert!(set.is_match(Path::new("Fixtures/Sample.swift")));
    }
}
