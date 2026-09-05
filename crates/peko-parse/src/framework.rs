//! Which framework built the app.
//!
//! A Flutter app ships an `.ipa` to Apple and every Apple rule applies to it,
//! so this is a property of a project rather than a fourth platform. It
//! decides which files hold the dependency graph and which rules mean
//! something different, and it never decides which store's rules apply.
//!
//! Detection reads files that are present, never file contents that could be
//! anything. A repository holding `pubspec.yaml` is a Dart project, and no
//! other framework writes that name.

use std::path::Path;

/// The frameworks this version knows.
///
/// Anything not on this list reads as `Unknown`, which is different from
/// `Native`. A project Peko cannot place is one it should say it cannot read,
/// rather than one it should check as though it were plain Swift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Framework {
    /// Plain Swift, Objective C, Kotlin, or Java.
    Native,
    Flutter,
    ReactNative,
    /// React Native under Expo, where `ios/` and `android/` are generated at
    /// build time and are usually absent from the repository.
    Expo,
    Capacitor,
    Maui,
    Unity,
    KotlinMultiplatform,
    /// Files that place the project in no known framework.
    Unknown,
}

impl Framework {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Framework::Native => "native",
            Framework::Flutter => "flutter",
            Framework::ReactNative => "react-native",
            Framework::Expo => "expo",
            Framework::Capacitor => "capacitor",
            Framework::Maui => "maui",
            Framework::Unity => "unity",
            Framework::KotlinMultiplatform => "kotlin-multiplatform",
            Framework::Unknown => "unknown",
        }
    }

    /// Whether this version reads the dependency graph of the framework.
    ///
    /// False means a report about this project would rest on a fraction of
    /// what it links, so the run says so rather than reporting a pass.
    #[must_use]
    pub fn dependencies_are_read(self) -> bool {
        matches!(
            self,
            Framework::Native
                | Framework::Flutter
                | Framework::ReactNative
                | Framework::Expo
                | Framework::Maui
                | Framework::KotlinMultiplatform
        )
    }
}

/// What the detector saw. Order matters: the first match wins, and the list
/// runs from the most specific name to the least.
const SIGNS: &[(&str, Framework)] = &[
    ("pubspec.yaml", Framework::Flutter),
    ("capacitor.config.json", Framework::Capacitor),
    ("capacitor.config.ts", Framework::Capacitor),
    ("app.config.js", Framework::Expo),
    ("app.config.ts", Framework::Expo),
    ("packages/manifest.json", Framework::Unity),
    ("projectsettings/projectsettings.asset", Framework::Unity),
];

/// Decide the framework from the paths a walk found.
///
/// `paths` are relative to the project root. Every comparison is lowercase,
/// because a case insensitive file system hands back whatever was typed.
#[must_use]
pub fn detect(paths: &[impl AsRef<Path>]) -> Framework {
    let names: Vec<String> = paths
        .iter()
        .map(|path| path.as_ref().to_string_lossy().to_ascii_lowercase())
        .collect();

    let has = |needle: &str| names.iter().any(|name| name.ends_with(needle));
    let any_named = |needle: &str| names.iter().any(|name| name.contains(needle));

    for (sign, framework) in SIGNS {
        if has(sign) {
            // Expo and bare React Native both hold package.json, and an Expo
            // project usually holds app.json too. The config file is the one
            // that separates them.
            return *framework;
        }
    }

    if has("app.json") && has("package.json") {
        return Framework::Expo;
    }
    if has("package.json") && (any_named("metro.config") || any_named("react-native.config")) {
        return Framework::ReactNative;
    }
    if names.iter().any(|name| name.ends_with(".csproj")) {
        return Framework::Maui;
    }
    if has("libs.versions.toml") && any_named("commonmain") {
        return Framework::KotlinMultiplatform;
    }
    if has("package.json") {
        // package.json with no native config and no framework marker is a web
        // project or a tool, not something this reads.
        return Framework::Unknown;
    }
    if has("info.plist") || has("androidmanifest.xml") || has("project.pbxproj") {
        return Framework::Native;
    }
    Framework::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn a_flutter_project_is_flutter_and_not_native() {
        // A Flutter repository holds both native manifests, which is exactly
        // what made detect_platform give up before this existed.
        let found = detect(&paths(&[
            "pubspec.yaml",
            "pubspec.lock",
            "ios/Runner/Info.plist",
            "android/app/src/main/AndroidManifest.xml",
        ]));
        assert_eq!(found, Framework::Flutter);
    }

    #[test]
    fn expo_is_not_read_as_bare_react_native() {
        let found = detect(&paths(&["package.json", "app.json", "App.tsx"]));
        assert_eq!(found, Framework::Expo);
    }

    #[test]
    fn bare_react_native_needs_its_own_marker() {
        let found = detect(&paths(&[
            "package.json",
            "metro.config.js",
            "ios/App/Info.plist",
        ]));
        assert_eq!(found, Framework::ReactNative);
    }

    #[test]
    fn a_plain_xcode_project_is_native() {
        assert_eq!(
            detect(&paths(&["App.xcodeproj/project.pbxproj", "App/Info.plist"])),
            Framework::Native
        );
    }

    #[test]
    fn something_unplaceable_is_unknown_rather_than_native() {
        // The dangerous answer is Native, because every native rule would run
        // against a project nobody read and report a pass.
        assert_eq!(
            detect(&paths(&["README.md", "Makefile"])),
            Framework::Unknown
        );
    }

    #[test]
    fn unity_and_capacitor_dependencies_are_known_to_be_unread() {
        assert!(!Framework::Unity.dependencies_are_read());
        assert!(!Framework::Capacitor.dependencies_are_read());
        assert!(!Framework::Unknown.dependencies_are_read());
        assert!(Framework::Flutter.dependencies_are_read());
    }
}
