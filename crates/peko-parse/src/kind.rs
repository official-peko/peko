//! Classification of project files by name.

use std::path::Path;

/// The kind of project file, decided by file name and parent directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileKind {
    InfoPlist,
    AndroidManifest,
    Entitlements,
    PrivacyManifest,
    XcodeProject,
    BuildGradle,
    PodfileLock,
    PackageResolved,
    GradleVersionCatalog,
    /// `pubspec.lock`, the resolved Dart package list for a Flutter project.
    PubspecLock,
    /// `pubspec.yaml`, which names the direct Dart dependencies.
    Pubspec,
    /// `package.json`, the direct npm dependencies of a React Native, Expo,
    /// or Capacitor project.
    PackageJson,
    /// An npm lockfile: `package-lock.json`, `yarn.lock`, or `pnpm-lock.yaml`.
    NpmLock,
    /// A .NET project file, which names its packages inline.
    CsProj,
    /// `packages.lock.json`, the resolved .NET package list.
    NuGetLock,
    /// `app.json` or `app.config.json`, where an Expo project keeps the
    /// configuration that a native project keeps in Info.plist.
    ExpoConfig,
    /// `capacitor.config.json`, which holds the server url a hybrid app loads.
    CapacitorConfig,
    Source,
    /// A privacy policy that ships in the repository.
    ///
    /// This is a separate kind rather than a source file. A policy is prose,
    /// and prose full of words like "we collect your birthday" would match
    /// the symbol scans and report findings against sentences.
    PolicyDocument,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::InfoPlist => "info_plist",
            FileKind::AndroidManifest => "android_manifest",
            FileKind::Entitlements => "entitlements",
            FileKind::PrivacyManifest => "privacy_manifest",
            FileKind::PolicyDocument => "policy_document",
            FileKind::XcodeProject => "xcode_project",
            FileKind::BuildGradle => "build_gradle",
            FileKind::PodfileLock => "podfile_lock",
            FileKind::PackageResolved => "package_resolved",
            FileKind::GradleVersionCatalog => "gradle_version_catalog",
            FileKind::PubspecLock => "pubspec_lock",
            FileKind::Pubspec => "pubspec",
            FileKind::PackageJson => "package_json",
            FileKind::NpmLock => "npm_lock",
            FileKind::CsProj => "csproj",
            FileKind::NuGetLock => "nuget_lock",
            FileKind::ExpoConfig => "expo_config",
            FileKind::CapacitorConfig => "capacitor_config",
            FileKind::Source => "source",
        }
    }
}

/// Source file extensions that the checker scans.
pub const SOURCE_EXTENSIONS: &[&str] = &[
    "swift", "m", "mm", "h", "hpp", "c", "cc", "cpp", "kt", "kts", "java", "js", "jsx", "ts",
    "tsx", "dart",
    // .NET MAUI writes its code in C# and its layout in XAML. Neither was
    // walked, so a MAUI project was read as a repository with no source in it.
    "cs", "xaml",
    // Two file kinds that hold policy and are not code. A StoreKit
    // configuration states the subscription period, and the store listing text
    // under fastlane holds the name, subtitle, and description that review
    // reads. Two rules named those files and neither could ever fire, because
    // the walker ignored them.
    "storekit", "txt",
];

/// Decide the kind of a file from its path. Returns `None` for a file the
/// checker ignores.
pub fn classify(path: &Path) -> Option<FileKind> {
    let name = path.file_name()?.to_str()?;
    let lower = name.to_ascii_lowercase();

    match lower.as_str() {
        "info.plist" => return Some(FileKind::InfoPlist),
        "androidmanifest.xml" => return Some(FileKind::AndroidManifest),
        "privacyinfo.xcprivacy" => return Some(FileKind::PrivacyManifest),
        "project.pbxproj" => return Some(FileKind::XcodeProject),
        "build.gradle" | "build.gradle.kts" => return Some(FileKind::BuildGradle),
        "podfile.lock" => return Some(FileKind::PodfileLock),
        "package.resolved" => return Some(FileKind::PackageResolved),
        "libs.versions.toml" => return Some(FileKind::GradleVersionCatalog),
        "pubspec.lock" => return Some(FileKind::PubspecLock),
        "pubspec.yaml" => return Some(FileKind::Pubspec),
        "package.json" => return Some(FileKind::PackageJson),
        "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml" => return Some(FileKind::NpmLock),
        "packages.lock.json" => return Some(FileKind::NuGetLock),
        "app.json" | "app.config.json" => return Some(FileKind::ExpoConfig),
        "capacitor.config.json" => return Some(FileKind::CapacitorConfig),
        _ => {}
    }

    if lower.ends_with(".entitlements") {
        return Some(FileKind::Entitlements);
    }
    if lower.ends_with(".csproj") {
        return Some(FileKind::CsProj);
    }

    let extension = path.extension()?.to_str()?.to_ascii_lowercase();

    // A privacy policy is found by name. Every other document with these
    // extensions stays out, so a README never joins the source scan.
    if matches!(extension.as_str(), "md" | "txt" | "html")
        && (lower.starts_with("privacy")
            || lower.contains("privacy-policy")
            || lower.contains("privacy_policy"))
    {
        return Some(FileKind::PolicyDocument);
    }

    if SOURCE_EXTENSIONS.contains(&extension.as_str()) {
        return Some(FileKind::Source);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn classifies_known_files() {
        let cases = [
            ("App/Info.plist", FileKind::InfoPlist),
            (
                "app/src/main/AndroidManifest.xml",
                FileKind::AndroidManifest,
            ),
            ("App/App.entitlements", FileKind::Entitlements),
            ("App/PrivacyInfo.xcprivacy", FileKind::PrivacyManifest),
            ("App.xcodeproj/project.pbxproj", FileKind::XcodeProject),
            ("app/build.gradle.kts", FileKind::BuildGradle),
            ("Podfile.lock", FileKind::PodfileLock),
            ("App/ContentView.swift", FileKind::Source),
            ("app/src/Main.kt", FileKind::Source),
        ];
        for (path, expected) in cases {
            assert_eq!(classify(&PathBuf::from(path)), Some(expected), "{path}");
        }
    }

    #[test]
    fn ignores_unknown_files() {
        assert_eq!(classify(&PathBuf::from("README.md")), None);
        assert_eq!(classify(&PathBuf::from("assets/logo.png")), None);
    }

    #[test]
    fn finds_a_privacy_policy_by_name() {
        for path in [
            "PRIVACY.md",
            "docs/privacy.txt",
            "web/privacy-policy.html",
            "legal/Privacy_Policy.md",
        ] {
            assert_eq!(
                classify(&PathBuf::from(path)),
                Some(FileKind::PolicyDocument),
                "{path}"
            );
        }
    }

    #[test]
    fn a_policy_document_never_joins_the_source_scan() {
        // Prose that says "we collect your date of birth" must not report as
        // code that collects a date of birth.
        assert_ne!(
            classify(&PathBuf::from("PRIVACY.md")),
            Some(FileKind::Source)
        );
        assert_eq!(classify(&PathBuf::from("CHANGELOG.md")), None);
        assert_eq!(classify(&PathBuf::from("docs/design.md")), None);
    }
}
