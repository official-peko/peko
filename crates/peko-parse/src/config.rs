//! Build configuration reading for `build.gradle` and `project.pbxproj`.
//!
//! Neither file has a stable public grammar, and `project.pbxproj` uses the
//! `OpenStep` property list format that the `plist` crate does not read. Both
//! parsers therefore work on text. They extract assignments only. A setting
//! that the parser cannot find reads as absent, never as wrong.

use crate::error::{ParseError, Result};
use regex::Regex;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

/// One assignment found in a configuration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingValue {
    /// The value with quotes removed.
    pub value: String,
    /// The one-based line number of the assignment.
    pub line: usize,
}

/// Every assignment found in one configuration file.
#[derive(Debug, Clone, Default)]
pub struct BuildSettings {
    values: BTreeMap<String, Vec<SettingValue>>,
}

impl BuildSettings {
    /// Every assignment of `key`, in file order.
    pub fn get(&self, key: &str) -> &[SettingValue] {
        self.values.get(key).map_or(&[], Vec::as_slice)
    }

    /// The first assignment of `key`.
    pub fn first(&self, key: &str) -> Option<&SettingValue> {
        self.get(key).first()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    fn insert(&mut self, key: String, value: SettingValue) {
        self.values.entry(key).or_default().push(value);
    }
}

fn gradle_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?m)^[ \t]*(?P<key>[A-Za-z_][A-Za-z0-9_]*)[ \t]*(?:=[ \t]*|\([ \t]*|[ \t]+)(?P<value>"[^"]*"|'[^']*'|[A-Za-z0-9_.+\-]+)[ \t]*\)?[ \t]*(?://.*)?$"#,
        )
        .expect("gradle regex is valid")
    })
}

fn pbxproj_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?m)^[ \t]*(?P<key>[A-Za-z_][A-Za-z0-9_]*)[ \t]*=[ \t]*(?P<value>"[^"\n]*"|[^;\n(]+);"#)
            .expect("pbxproj regex is valid")
    })
}

/// Read assignments out of `build.gradle` or `build.gradle.kts`.
pub fn parse_gradle(text: &str) -> BuildSettings {
    extract(text, gradle_regex())
}

/// True when a build script applies the Android application plugin.
///
/// A repository holds one build script per module. Only the module that
/// applies this plugin becomes the app that ships. A library module sets its
/// own target level, and that level says nothing about the app.
pub fn declares_android_application(text: &str) -> bool {
    text.contains("com.android.application")
}

/// Read build settings out of `project.pbxproj`.
pub fn parse_pbxproj(text: &str) -> BuildSettings {
    extract(text, pbxproj_regex())
}

fn extract(text: &str, regex: &Regex) -> BuildSettings {
    let mut settings = BuildSettings::default();
    let mut line_starts = vec![0usize];
    line_starts.extend(text.match_indices('\n').map(|(index, _)| index + 1));

    for capture in regex.captures_iter(text) {
        let key = capture.name("key").expect("key group").as_str().to_string();
        let raw = capture.name("value").expect("value group");
        let value = unquote(raw.as_str().trim());
        if value.is_empty() {
            continue;
        }
        let line = line_starts.partition_point(|start| *start <= raw.start());
        settings.insert(key, SettingValue { value, line });
    }
    settings
}

fn unquote(text: &str) -> String {
    let bytes = text.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        return text[1..text.len() - 1].to_string();
    }
    text.to_string()
}

/// Read and parse a configuration file.
pub fn read_config(path: &Path, gradle: bool) -> Result<BuildSettings> {
    let text = std::fs::read_to_string(path).map_err(|source| ParseError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(if gradle {
        parse_gradle(&text)
    } else {
        parse_pbxproj(&text)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRADLE_APP: &str = r#"
plugins { id("com.android.application") }

android {
    namespace = "com.example.app"
    compileSdk 35

    defaultConfig {
        applicationId "com.example.app"
        minSdk = 24
        targetSdk(35)
        versionCode 12
    }
}

dependencies {
    implementation "com.squareup.okhttp3:okhttp:4.12.0"
}
"#;

    const PBXPROJ: &str = r#"
        buildSettings = {
            IPHONEOS_DEPLOYMENT_TARGET = 16.0;
            PRODUCT_BUNDLE_IDENTIFIER = com.example.app;
            SWIFT_VERSION = 5.0;
            CODE_SIGN_ENTITLEMENTS = "App/App.entitlements";
        };
"#;

    #[test]
    fn reads_every_gradle_assignment_form() {
        let settings = parse_gradle(GRADLE_APP);
        assert_eq!(
            settings.first("namespace").unwrap().value,
            "com.example.app"
        );
        assert_eq!(settings.first("compileSdk").unwrap().value, "35");
        assert_eq!(settings.first("minSdk").unwrap().value, "24");
        assert_eq!(settings.first("targetSdk").unwrap().value, "35");
        assert_eq!(
            settings.first("applicationId").unwrap().value,
            "com.example.app"
        );
    }

    #[test]
    fn records_gradle_line_numbers() {
        let settings = parse_gradle(GRADLE_APP);
        assert_eq!(settings.first("compileSdk").unwrap().line, 6);
    }

    #[test]
    fn reads_pbxproj_settings_and_strips_quotes() {
        let settings = parse_pbxproj(PBXPROJ);
        assert_eq!(
            settings.first("IPHONEOS_DEPLOYMENT_TARGET").unwrap().value,
            "16.0"
        );
        assert_eq!(
            settings.first("PRODUCT_BUNDLE_IDENTIFIER").unwrap().value,
            "com.example.app"
        );
        assert_eq!(
            settings.first("CODE_SIGN_ENTITLEMENTS").unwrap().value,
            "App/App.entitlements"
        );
    }

    #[test]
    fn the_application_plugin_is_detected() {
        assert!(declares_android_application(GRADLE_APP));
        assert!(!declares_android_application(
            "plugins { id(\"com.android.library\") }\nandroid { compileSdk = 35 }\n"
        ));
    }

    #[test]
    fn absent_settings_read_as_absent() {
        let settings = parse_gradle(GRADLE_APP);
        assert!(!settings.contains("targetSdkVersion"));
        assert!(settings.first("targetSdkVersion").is_none());
    }
}
