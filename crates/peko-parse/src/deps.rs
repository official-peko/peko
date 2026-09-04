//! Direct dependency extraction from lockfiles and build scripts.
//!
//! V1 reads direct dependencies only. Transitive dependencies stay out of
//! scope, and the report says so.

use crate::error::{ParseError, Result};
use peko_rules::Ecosystem;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Whether a dependency reaches the shipped app.
///
/// A test dependency runs on a build machine and never reaches a user, so it
/// collects nothing in production. Counting one on a privacy form overstates
/// what the app does, and counting it as an unknown package understates how
/// much of the real dependency list the knowledge base covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// The dependency ships inside the app.
    #[default]
    Ships,
    /// The dependency builds or runs tests only.
    TestOnly,
}

/// One direct dependency of the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    /// The ecosystem qualified id, for example `cocoapods:AFNetworking`.
    pub package_id: String,
    /// The bare package name.
    pub name: String,
    pub ecosystem: Ecosystem,
    /// The version constraint or resolved version, when the file states one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The file that declares the dependency.
    pub declared_in: PathBuf,
    /// The one-based line number of the declaration, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// Whether the dependency reaches the shipped app.
    ///
    /// Only the Gradle parser reads this today, because only Gradle names the
    /// configuration on the same line as the coordinate. A `CocoaPods` or Swift
    /// test target is a separate block, and this reports `Ships` for those
    /// rather than guess.
    #[serde(default)]
    pub scope: Scope,
}

impl Dependency {
    fn new(
        ecosystem: Ecosystem,
        name: impl Into<String>,
        version: Option<String>,
        declared_in: &Path,
        line: Option<usize>,
    ) -> Self {
        let name = name.into();
        Self {
            package_id: format!("{}:{}", ecosystem.as_str(), name),
            name,
            ecosystem,
            version,
            declared_in: declared_in.to_path_buf(),
            line,
            scope: Scope::Ships,
        }
    }

    fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
}

/// The scope a Gradle configuration name implies.
///
/// `debugImplementation` ships, in a debug build that a tester installs. Only
/// the test configurations never reach a device a user holds.
fn scope_for_configuration(configuration: &str) -> Scope {
    if configuration.starts_with("test") || configuration.starts_with("androidTest") {
        Scope::TestOnly
    } else {
        Scope::Ships
    }
}

/// Parse the `DEPENDENCIES:` section of a `Podfile.lock`.
///
/// The `PODS:` section lists resolved transitive pods. V1 reports direct pods
/// only, so this reads `DEPENDENCIES:`.
pub fn parse_podfile_lock(path: &Path, text: &str) -> Vec<Dependency> {
    let mut out = Vec::new();
    let mut inside = false;
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_end();
        if trimmed.starts_with("DEPENDENCIES:") {
            inside = true;
            continue;
        }
        if inside {
            if !trimmed.starts_with(' ') && !trimmed.is_empty() {
                break;
            }
            let Some(entry) = trimmed.trim().strip_prefix("- ") else {
                continue;
            };
            let entry = entry.trim().trim_matches('"');
            let (name, version) = split_paren_version(entry);
            if name.is_empty() {
                continue;
            }
            out.push(Dependency::new(
                Ecosystem::Cocoapods,
                name,
                version,
                path,
                Some(index + 1),
            ));
        }
    }
    out
}

fn split_paren_version(entry: &str) -> (String, Option<String>) {
    match entry.split_once(" (") {
        Some((name, rest)) => (
            name.trim().to_string(),
            Some(rest.trim_end_matches([')', ':']).trim().to_string()),
        ),
        None => (entry.trim_end_matches(':').trim().to_string(), None),
    }
}

/// Parse `Package.resolved`. Both the version 1 and the version 2 or later
/// layouts are read.
pub fn parse_package_resolved(path: &Path, text: &str) -> Result<Vec<Dependency>> {
    let root: serde_json::Value =
        serde_json::from_str(text).map_err(|source| ParseError::Json {
            path: path.to_path_buf(),
            source,
        })?;

    let pins = root
        .get("pins")
        .or_else(|| root.get("object").and_then(|object| object.get("pins")))
        .and_then(serde_json::Value::as_array);
    let Some(pins) = pins else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for pin in pins {
        let name = pin
            .get("identity")
            .and_then(serde_json::Value::as_str)
            .or_else(|| pin.get("package").and_then(serde_json::Value::as_str));
        let Some(name) = name else { continue };
        let version = pin
            .get("state")
            .and_then(|state| state.get("version"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        out.push(Dependency::new(
            Ecosystem::SwiftPackage,
            name,
            version,
            path,
            None,
        ));
    }
    Ok(out)
}

fn gradle_dependency_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?m)^[ \t]*(?P<config>implementation|api|compileOnly|runtimeOnly|testImplementation|androidTestImplementation|debugImplementation|releaseImplementation|ksp|kapt|annotationProcessor)[ \t]*[\( \t]*["'](?P<coord>[^"']+)["']"#,
        )
        .expect("gradle dependency regex is valid")
    })
}

/// Read dependency declarations out of `build.gradle` or `build.gradle.kts`.
///
/// Version catalog references such as `libs.androidx.core` carry no
/// coordinate, so this parser skips them. `parse_version_catalog` reads those.
pub fn parse_gradle_dependencies(path: &Path, text: &str) -> Vec<Dependency> {
    let mut line_starts = vec![0usize];
    line_starts.extend(text.match_indices('\n').map(|(index, _)| index + 1));

    let mut out = Vec::new();
    for capture in gradle_dependency_regex().captures_iter(text) {
        let coord = capture.name("coord").expect("coord group");
        let parts: Vec<&str> = coord.as_str().split(':').collect();
        if parts.len() < 2 {
            continue;
        }
        let name = format!("{}:{}", parts[0], parts[1]);
        let version = parts.get(2).map(|value| (*value).to_string());
        let line = line_starts.partition_point(|start| *start <= coord.start());
        let scope = capture.name("config").map_or(Scope::Ships, |value| {
            scope_for_configuration(value.as_str())
        });
        out.push(
            Dependency::new(Ecosystem::Gradle, name, version, path, Some(line)).with_scope(scope),
        );
    }
    out
}

/// Read the `[libraries]` table of a Gradle version catalog.
pub fn parse_version_catalog(path: &Path, text: &str) -> Vec<Dependency> {
    let mut out = Vec::new();
    let mut inside = false;
    let mut group: Option<String> = None;
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == "[libraries]";
            continue;
        }
        if !inside || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((_, value)) = trimmed.split_once('=') else {
            continue;
        };
        let value = value.trim();

        if let Some(module) = field(value, "module") {
            push_coordinate(&mut out, &module, path, index + 1);
            continue;
        }
        let name_field = field(value, "name");
        group = field(value, "group").or(group.take());
        if let (Some(group_value), Some(artifact)) = (group.as_ref(), name_field) {
            push_coordinate(
                &mut out,
                &format!("{group_value}:{artifact}"),
                path,
                index + 1,
            );
        }
        group = None;
    }
    out
}

fn field(text: &str, key: &str) -> Option<String> {
    let marker = format!("{key} =");
    let start = text
        .find(&marker)
        .or_else(|| text.find(&format!("{key}=")))?;
    let rest = &text[start..];
    let open = rest.find('"')?;
    let tail = &rest[open + 1..];
    let close = tail.find('"')?;
    Some(tail[..close].to_string())
}

fn push_coordinate(out: &mut Vec<Dependency>, coordinate: &str, path: &Path, line: usize) {
    let parts: Vec<&str> = coordinate.split(':').collect();
    if parts.len() < 2 {
        return;
    }
    out.push(Dependency::new(
        Ecosystem::Gradle,
        format!("{}:{}", parts[0], parts[1]),
        parts.get(2).map(|value| (*value).to_string()),
        path,
        Some(line),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_direct_pods_only() {
        let text = r"PODS:
  - AFNetworking (4.0.1):
    - AFNetworking/Serialization (= 4.0.1)
  - FirebaseCore (10.24.0)
  - TransitiveOnly (1.0.0)

DEPENDENCIES:
  - AFNetworking (~> 4.0)
  - FirebaseCore

SPEC CHECKSUMS:
  AFNetworking: abc123
";
        let deps = parse_podfile_lock(Path::new("Podfile.lock"), text);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].package_id, "cocoapods:AFNetworking");
        assert_eq!(deps[0].version.as_deref(), Some("~> 4.0"));
        assert_eq!(deps[1].package_id, "cocoapods:FirebaseCore");
        assert!(deps[1].version.is_none());
    }

    #[test]
    fn reads_package_resolved_v2() {
        let text = r#"{
  "pins": [
    {
      "identity": "alamofire",
      "kind": "remoteSourceControl",
      "location": "https://github.com/Alamofire/Alamofire.git",
      "state": { "revision": "abc", "version": "5.9.1" }
    }
  ],
  "version": 2
}"#;
        let deps = parse_package_resolved(Path::new("Package.resolved"), text).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].package_id, "swift-package:alamofire");
        assert_eq!(deps[0].version.as_deref(), Some("5.9.1"));
    }

    #[test]
    fn reads_package_resolved_v1() {
        let text = r#"{ "object": { "pins": [ { "package": "Alamofire",
          "state": { "version": "5.4.0" } } ] }, "version": 1 }"#;
        let deps = parse_package_resolved(Path::new("Package.resolved"), text).unwrap();
        assert_eq!(deps[0].package_id, "swift-package:Alamofire");
    }

    #[test]
    fn reads_gradle_dependencies() {
        let text = r#"
dependencies {
    implementation "com.squareup.okhttp3:okhttp:4.12.0"
    implementation("com.google.firebase:firebase-analytics:21.6.1")
    testImplementation 'junit:junit:4.13.2'
    implementation(libs.androidx.core.ktx)
}
"#;
        let deps = parse_gradle_dependencies(Path::new("build.gradle.kts"), text);
        let scope_of = |name: &str| {
            deps.iter()
                .find(|d| d.name == name)
                .map(|d| d.scope)
                .expect("the dependency parsed")
        };
        // A test configuration must not read as shipped. The whole privacy
        // form depends on this: junit ships nowhere and collects nothing.
        assert_eq!(scope_of("junit:junit"), Scope::TestOnly);
        assert_eq!(scope_of("com.squareup.okhttp3:okhttp"), Scope::Ships);
        assert_eq!(
            scope_of("com.google.firebase:firebase-analytics"),
            Scope::Ships
        );
        let ids: Vec<&str> = deps.iter().map(|d| d.package_id.as_str()).collect();
        assert!(ids.contains(&"gradle:com.squareup.okhttp3:okhttp"));
        assert!(ids.contains(&"gradle:com.google.firebase:firebase-analytics"));
        assert!(ids.contains(&"gradle:junit:junit"));
        assert_eq!(deps.len(), 3);
    }

    #[test]
    fn reads_a_version_catalog() {
        let text = r#"
[versions]
okhttp = "4.12.0"

[libraries]
okhttp = { module = "com.squareup.okhttp3:okhttp", version.ref = "okhttp" }
core-ktx = { group = "androidx.core", name = "core-ktx", version = "1.13.1" }
"#;
        let deps = parse_version_catalog(Path::new("libs.versions.toml"), text);
        let ids: Vec<&str> = deps.iter().map(|d| d.package_id.as_str()).collect();
        assert!(ids.contains(&"gradle:com.squareup.okhttp3:okhttp"));
        assert!(ids.contains(&"gradle:androidx.core:core-ktx"));
    }
}
