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
            FileKind::Source => "source",
        }
    }
}

/// Source file extensions that the checker scans.
pub const SOURCE_EXTENSIONS: &[&str] = &[
    "swift", "m", "mm", "h", "hpp", "c", "cc", "cpp", "kt", "kts", "java", "js", "jsx", "ts",
    "tsx", "dart",
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
        _ => {}
    }

    if lower.ends_with(".entitlements") {
        return Some(FileKind::Entitlements);
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
