//! Collecting the files a lint request carries.
//!
//! This is the one thing the client knows how to do that the server does not.
//! It is not analysis: the request shape names the files, and this finds them
//! by name and reads them. Nothing here decides whether anything is wrong.

use base64::Engine as _;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The files a lint request carries, in the shape the API takes.
#[derive(Debug, Default, Serialize)]
pub struct Files {
    pub manifests: Manifests,
    pub configs: Configs,
    pub changed_sources: Vec<SourceFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lockfile: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct Manifests {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info_plist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub android_manifest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privacy_manifest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entitlements: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct Configs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xcode_project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_gradle: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SourceFile {
    pub path: String,
    pub content: String,
}

/// The most source a lint request carries.
///
/// A lint runs on what a commit touched. A request far past this is an audit
/// in the wrong place, and the server refuses it anyway.
pub const MAX_SOURCE_BYTES: usize = 6 * 1024 * 1024;

/// The extensions a source file has.
const SOURCE_EXTENSIONS: &[&str] = &[
    "swift", "m", "mm", "h", "kt", "kts", "java", "js", "jsx", "ts", "tsx", "dart",
];

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn read(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|bytes| encode(&bytes))
}

/// Find the files, read them, and say what was left out.
///
/// `changed` names the paths a commit touched. An empty list means the whole
/// project, which is what a first run does.
pub fn collect(root: &Path, changed: &[PathBuf]) -> (Files, Vec<String>) {
    let all = crate::config::walk(root, 6);
    let mut files = Files::default();
    let mut skipped = Vec::new();

    for path in &all {
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        match name {
            "Info.plist" if files.manifests.info_plist.is_none() => {
                files.manifests.info_plist = read(path);
            }
            "AndroidManifest.xml" if files.manifests.android_manifest.is_none() => {
                // Only the manifest the application module ships. A library
                // manifest is not the one the store reads.
                if path.to_string_lossy().contains("src/main") {
                    files.manifests.android_manifest = read(path);
                }
            }
            "PrivacyInfo.xcprivacy" if files.manifests.privacy_manifest.is_none() => {
                files.manifests.privacy_manifest = read(path);
            }
            "project.pbxproj" if files.configs.xcode_project.is_none() => {
                files.configs.xcode_project = read(path);
            }
            "build.gradle" | "build.gradle.kts" if files.configs.build_gradle.is_none() => {
                files.configs.build_gradle = read(path);
            }
            "Podfile.lock" | "Package.resolved" if files.lockfile.is_none() => {
                files.lockfile = read(path);
            }
            _ => {
                if path.extension().is_some_and(|ext| ext == "entitlements")
                    && files.manifests.entitlements.is_none()
                {
                    files.manifests.entitlements = read(path);
                }
            }
        }
    }

    // The sources, either the ones a commit touched or all of them.
    let wanted: Vec<PathBuf> = if changed.is_empty() {
        all.iter()
            .filter(|path| {
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| SOURCE_EXTENSIONS.contains(&ext))
            })
            .cloned()
            .collect()
    } else {
        changed.to_vec()
    };

    let mut total = 0usize;
    for path in wanted {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        if total + bytes.len() > MAX_SOURCE_BYTES {
            skipped.push(relative);
            continue;
        }
        total += bytes.len();
        files.changed_sources.push(SourceFile {
            path: relative,
            content: encode(&bytes),
        });
    }
    (files, skipped)
}

/// The files a commit touched, when this is a git repository.
///
/// A lint reads what changed, and asking git is cheaper and more accurate than
/// guessing from timestamps. Outside a repository the answer is nothing, which
/// makes the caller send everything.
pub fn changed_files(root: &Path, against: &str) -> Vec<PathBuf> {
    let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--name-only", "--diff-filter=d", against])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| root.join(line.trim()))
        .filter(|path| path.is_file())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("peko-gather-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        path
    }

    fn write(root: &Path, relative: &str, text: &str) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn it_finds_each_file_the_request_names() {
        let root = scratch("ios");
        write(&root, "App/Info.plist", "<plist/>");
        write(&root, "App/PrivacyInfo.xcprivacy", "<plist/>");
        write(&root, "App/App.entitlements", "<plist/>");
        write(&root, "App.xcodeproj/project.pbxproj", "// pbxproj");
        write(&root, "Podfile.lock", "PODS:");
        write(&root, "App/View.swift", "import UIKit");

        let (files, skipped) = collect(&root, &[]);
        assert!(files.manifests.info_plist.is_some());
        assert!(files.manifests.privacy_manifest.is_some());
        assert!(files.manifests.entitlements.is_some());
        assert!(files.configs.xcode_project.is_some());
        assert!(files.lockfile.is_some());
        assert_eq!(files.changed_sources.len(), 1);
        assert_eq!(files.changed_sources[0].path, "App/View.swift");
        assert!(skipped.is_empty());
    }

    /// A library manifest is not the one the store reads.
    #[test]
    fn it_takes_the_application_manifest_and_not_a_library_one() {
        let root = scratch("android");
        write(
            &root,
            "core/AndroidManifest.xml",
            "<manifest>library</manifest>",
        );
        write(
            &root,
            "app/src/main/AndroidManifest.xml",
            "<manifest>app</manifest>",
        );
        let (files, _) = collect(&root, &[]);
        let encoded = files.manifests.android_manifest.expect("a manifest");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("app"));
    }

    #[test]
    fn a_named_change_set_replaces_the_whole_project() {
        let root = scratch("changed");
        write(&root, "App/One.swift", "one");
        write(&root, "App/Two.swift", "two");
        let (files, _) = collect(&root, &[root.join("App/Two.swift")]);
        assert_eq!(files.changed_sources.len(), 1);
        assert_eq!(files.changed_sources[0].path, "App/Two.swift");
    }

    /// A request that runs past the limit says what it left out, rather than
    /// sending a smaller answer and looking complete.
    #[test]
    fn what_does_not_fit_is_named() {
        let root = scratch("large");
        let big = "x".repeat(MAX_SOURCE_BYTES);
        write(&root, "App/Big.swift", &big);
        write(&root, "App/Small.swift", "small");
        let (files, skipped) = collect(&root, &[]);
        assert_eq!(files.changed_sources.len() + skipped.len(), 2);
        assert!(!skipped.is_empty(), "nothing was reported as left out");
    }

    #[test]
    fn a_directory_that_is_not_a_repository_reports_no_changes() {
        let root = scratch("norepo");
        assert!(changed_files(&root, "HEAD").is_empty());
    }
}
