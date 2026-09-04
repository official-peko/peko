//! The Gradle module graph for an Android project.
//!
//! An Android build is a tree of modules. One module applies
//! `com.android.application` and builds the APK the store gets. The others
//! apply `com.android.library` and ship inside it. Under each module, Gradle
//! splits files into source sets: `src/main` ships, `src/test` and
//! `src/androidTest` never do, and a debug build type never reaches the store.
//!
//! Before this module the checker read a build file as loose text. It looked
//! for the string `com.android.application`, which the version catalog form
//! `alias(libs.plugins.android.application)` does not contain, so large real
//! apps reported no application module at all and every build setting check
//! passed for the wrong reason. This graph resolves the plugin properly and
//! gives Android the same file ownership answer that [`crate::pbxproj`] gives
//! iOS.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

/// What a Gradle module builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind {
    /// `com.android.application`. This module builds the shipped APK.
    Application,
    /// `com.android.library` or `com.android.dynamic-feature`. Its code ships
    /// inside an application module.
    Library,
    /// `com.android.test`. The whole module is a test and never ships.
    Test,
    /// A plain JVM module, or a build file with no plugin this parser knows.
    Other,
}

impl ModuleKind {
    /// Does any code in this module reach the store?
    pub fn ships(self) -> bool {
        !matches!(self, Self::Test)
    }
}

/// One source set directory under a module, such as `src/main`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSet {
    /// The Gradle name, such as `main`, `androidTest`, or `wordpressDebug`.
    pub name: String,
    /// The directory, relative to the project root.
    pub dir: PathBuf,
    /// Does a file in this directory reach the store?
    pub ships: bool,
}

/// One Gradle module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradleModule {
    /// The module directory, relative to the project root. The root module is
    /// the empty path.
    pub dir: PathBuf,
    /// The build file, relative to the project root.
    pub build_file: PathBuf,
    pub kind: ModuleKind,
    pub source_sets: Vec<SourceSet>,
}

/// Every module in one Android project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GradleProject {
    /// Modules, sorted by directory depth, deepest first, so the first match
    /// on a path prefix is the owning module.
    pub modules: Vec<GradleModule>,
}

/// One build file to place in the graph.
#[derive(Debug, Clone)]
pub struct ModuleInput {
    /// The build file path, relative to the project root.
    pub build_file: PathBuf,
    /// The build file text.
    pub text: String,
}

/// The build files a checker must read.
///
/// A test module, and a source set that does not ship, are both left out.
const ANDROID_APPLICATION: &str = "com.android.application";
const ANDROID_LIBRARY: &str = "com.android.library";
const ANDROID_DYNAMIC_FEATURE: &str = "com.android.dynamic-feature";
const ANDROID_TEST: &str = "com.android.test";

/// Source set name prefixes that never reach the store.
///
/// A variant suffix follows the prefix in upper case, as in `testDebug` or
/// `androidTestPlay`. The upper case test keeps a product flavor named
/// `testing` out, because dropping a shipping file hides real findings.
const TEST_SOURCE_SETS: [&str; 4] = ["test", "androidTest", "testFixtures", "screenshotTest"];

/// Build the module graph.
///
/// `all_paths` holds every file the walker found, relative to the project
/// root. Source sets come from those paths, so this function reads no files.
pub fn build_gradle_project(
    modules: &[ModuleInput],
    catalog: Option<&str>,
    all_paths: &[PathBuf],
) -> GradleProject {
    let aliases = catalog.map(parse_catalog_plugins).unwrap_or_default();

    let mut built: Vec<GradleModule> = modules
        .iter()
        .map(|input| {
            let dir = input
                .build_file
                .parent()
                .unwrap_or(Path::new(""))
                .to_path_buf();
            let stripped = strip_comments(&input.text);
            let kind = if is_test_module(&dir) {
                ModuleKind::Test
            } else {
                classify_module(&stripped, &aliases)
            };
            let debuggable = debuggable_build_types(&stripped);
            GradleModule {
                dir,
                build_file: input.build_file.clone(),
                kind,
                source_sets: Vec::new(),
            }
            .with_source_sets(all_paths, &debuggable)
        })
        .collect();

    // Deepest first, so `module_for` finds the closest module and not the
    // root module that contains every path.
    built.sort_by(|a, b| {
        b.dir
            .components()
            .count()
            .cmp(&a.dir.components().count())
            .then_with(|| a.dir.cmp(&b.dir))
    });
    GradleProject { modules: built }
}

impl GradleModule {
    fn with_source_sets(mut self, all_paths: &[PathBuf], debuggable: &BTreeSet<String>) -> Self {
        let mut names: BTreeSet<String> = BTreeSet::new();
        for path in all_paths {
            let Ok(rest) = path.strip_prefix(&self.dir) else {
                continue;
            };
            let mut parts = rest.components().map(std::path::Component::as_os_str);
            if parts.next().and_then(std::ffi::OsStr::to_str) != Some("src") {
                continue;
            }
            if let Some(name) = parts.next().and_then(std::ffi::OsStr::to_str) {
                names.insert(name.to_string());
            }
        }
        self.source_sets = names
            .into_iter()
            .map(|name| {
                let ships =
                    self.kind.ships() && !is_test_source_set(&name) && !debuggable.contains(&name);
                SourceSet {
                    dir: self.dir.join("src").join(&name),
                    name,
                    ships,
                }
            })
            .collect();
        self
    }
}

impl GradleProject {
    /// The module that owns `path`, or `None` when no build file covers it.
    pub fn module_for(&self, path: &Path) -> Option<&GradleModule> {
        self.modules
            .iter()
            .find(|module| path.starts_with(&module.dir))
    }

    /// Every module that builds a shipped APK.
    pub fn application_modules(&self) -> impl Iterator<Item = &GradleModule> {
        self.modules
            .iter()
            .filter(|module| module.kind == ModuleKind::Application)
    }

    /// Does `path` reach the store?
    ///
    /// A path no module claims ships, because a wrong guess must not hide a
    /// real finding.
    pub fn ships(&self, path: &Path) -> bool {
        let Some(module) = self.module_for(path) else {
            return true;
        };
        if !module.kind.ships() {
            return false;
        }
        module
            .source_sets
            .iter()
            .find(|set| path.starts_with(&set.dir))
            .is_none_or(|set| set.ships)
    }

    /// Every path in `all_paths` that never reaches the store.
    pub fn non_shipping_files(&self, all_paths: &[PathBuf]) -> BTreeSet<PathBuf> {
        all_paths
            .iter()
            .filter(|path| !self.ships(path))
            .cloned()
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }
}

/// Read the plugin aliases out of a Gradle version catalog.
///
/// Only the `[plugins]` table counts. The `[libraries]` table often holds the
/// same alias name for the plugin artifact, which is a different thing.
fn parse_catalog_plugins(text: &str) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    let mut in_plugins = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_plugins = line.starts_with("[plugins]");
            continue;
        }
        if !in_plugins {
            continue;
        }
        let Some((alias, rest)) = line.split_once('=') else {
            continue;
        };
        if let Some(id) = plugin_id_field(rest) {
            found.insert(normalize_alias(alias.trim()), id);
        }
    }
    found
}

/// The `id = "..."` field of a catalog plugin entry.
///
/// The entry may also be the short form `alias = "com.example:1.0"`.
fn plugin_id_field(rest: &str) -> Option<String> {
    static FIELD: OnceLock<Regex> = OnceLock::new();
    let field = FIELD
        .get_or_init(|| Regex::new(r#"\bid\s*=\s*['"]([^'"]+)['"]"#).expect("plugin id regex"));
    if let Some(capture) = field.captures(rest) {
        return Some(capture[1].to_string());
    }
    let short = rest.trim().trim_matches(['"', '\'']);
    if !short.starts_with('{') && short.contains('.') {
        return Some(short.split(':').next().unwrap_or(short).to_string());
    }
    None
}

/// Gradle treats `-`, `_`, and `.` in an alias as the same separator.
fn normalize_alias(alias: &str) -> String {
    alias.replace(['-', '_'], ".")
}

/// Every plugin id the build file applies.
///
/// A declaration marked `apply false` is skipped. A root build file lists
/// every plugin that way, and counting those would make the root module look
/// like the application module.
pub fn plugin_ids(text: &str, aliases: &BTreeMap<String, String>) -> BTreeSet<String> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(concat!(
            r#"(?:\bid\s*\(?\s*['"](?P<id>[^'"]+)['"]"#,
            r#"|\bapply\s+plugin\s*:\s*['"](?P<applied>[^'"]+)['"]"#,
            r#"|\balias\s*[\(\s]\s*[A-Za-z_][A-Za-z0-9_]*\.plugins\.(?P<alias>[A-Za-z0-9_.]+))"#,
        ))
        .expect("plugin regex")
    });

    let mut found = BTreeSet::new();
    for capture in pattern.captures_iter(text) {
        let whole = capture.get(0).expect("whole match");
        if declaration_is_not_applied(text, whole.end()) {
            continue;
        }
        let id = if let Some(value) = capture.name("id").or_else(|| capture.name("applied")) {
            value.as_str().to_string()
        } else if let Some(value) = capture.name("alias") {
            let key = normalize_alias(value.as_str());
            match aliases.get(&key) {
                Some(resolved) => resolved.clone(),
                None => continue,
            }
        } else {
            continue;
        };
        found.insert(id);
    }
    found
}

/// Does `apply false` follow the declaration that ends at `offset`?
fn declaration_is_not_applied(text: &str, offset: usize) -> bool {
    static APPLY_FALSE: OnceLock<Regex> = OnceLock::new();
    let rest = &text[offset..];
    let line = rest.split('\n').next().unwrap_or(rest);
    APPLY_FALSE
        .get_or_init(|| Regex::new(r"\bapply\s*[\(\s=]\s*false").expect("apply false regex"))
        .is_match(line)
}

/// True when the module directory name says the module is test scaffolding.
///
/// A module can apply `com.android.library` and still exist only so that the
/// instrumentation tests have somewhere to share code. Google I/O carries
/// `androidTest-shared` exactly that way, and its manifest tripped a location
/// rule for a permission no user ever receives.
///
/// The match is anchored rather than a substring, so a module called `latest`
/// or `contest` keeps shipping.
fn is_test_module(dir: &Path) -> bool {
    const EXACT: [&str; 6] = [
        "test",
        "tests",
        "testing",
        "testutils",
        "testlib",
        "fixtures",
    ];
    let Some(name) = dir.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    if EXACT.contains(&lower.as_str()) {
        return true;
    }
    lower.starts_with("androidtest")
        || lower.starts_with("test-")
        || lower.starts_with("testing-")
        || lower.ends_with("-test")
        || lower.ends_with("-tests")
        || lower.ends_with("-testing")
        || lower.ends_with("-testfixtures")
        || lower.ends_with("-testutils")
}

fn classify_module(text: &str, aliases: &BTreeMap<String, String>) -> ModuleKind {
    let plugins = plugin_ids(text, aliases);
    if plugins.contains(ANDROID_APPLICATION) {
        return ModuleKind::Application;
    }
    if plugins.contains(ANDROID_TEST) {
        return ModuleKind::Test;
    }
    if plugins.contains(ANDROID_LIBRARY) || plugins.contains(ANDROID_DYNAMIC_FEATURE) {
        return ModuleKind::Library;
    }
    ModuleKind::Other
}

/// The build types that produce a debuggable build.
///
/// A debuggable build never reaches the store, so a manifest or a setting in
/// its source set is not a compliance risk. Debug source sets often turn on
/// cleartext traffic and add permissions that a release build does not have.
pub fn debuggable_build_types(text: &str) -> BTreeSet<String> {
    static ENTRY: OnceLock<Regex> = OnceLock::new();
    static DEBUGGABLE: OnceLock<Regex> = OnceLock::new();
    let mut found = BTreeSet::new();
    let Some(block) = named_block(text, "buildTypes") else {
        // A module with no `buildTypes` block still gets the Android default.
        found.insert("debug".to_string());
        return found;
    };

    let entry = ENTRY.get_or_init(|| {
        Regex::new(r#"(?m)^\s*(?:create\s*\(\s*['"](?P<created>[^'"]+)['"]\s*\)|getByName\s*\(\s*['"](?P<named>[^'"]+)['"]\s*\)|(?P<bare>[A-Za-z_][A-Za-z0-9_]*))\s*\{"#)
            .expect("build type regex")
    });
    let debuggable = DEBUGGABLE.get_or_init(|| {
        Regex::new(r"\b(?:is)?[Dd]ebuggable\s*[=\s]\s*true").expect("debuggable regex")
    });

    for capture in entry.captures_iter(&block) {
        let name = capture
            .name("created")
            .or_else(|| capture.name("named"))
            .or_else(|| capture.name("bare"))
            .expect("build type name")
            .as_str();
        let open = capture.get(0).expect("whole match").end();
        let body = balanced_body(&block, open - 1).unwrap_or_default();
        if name == "debug" || debuggable.is_match(&body) {
            found.insert(name.to_string());
        }
    }
    if found.is_empty() {
        found.insert("debug".to_string());
    }
    found
}

/// The body of the first `name { ... }` block in `text`.
fn named_block(text: &str, name: &str) -> Option<String> {
    let pattern = Regex::new(&format!(r"\b{}\s*\{{", regex::escape(name))).ok()?;
    let start = pattern.find(text)?.end() - 1;
    balanced_body(text, start)
}

/// The text between the brace at `open` and the brace that closes it.
fn balanced_body(text: &str, open: usize) -> Option<String> {
    let bytes = text.as_bytes();
    if bytes.get(open) != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    for (offset, byte) in text.bytes().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[open + 1..offset].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn is_test_source_set(name: &str) -> bool {
    TEST_SOURCE_SETS.iter().any(|base| {
        name == *base
            || (name.starts_with(base)
                && name[base.len()..]
                    .chars()
                    .next()
                    .is_some_and(char::is_uppercase))
    })
}

/// Replace comment text with spaces, keeping every byte offset.
///
/// A `//` inside a quoted string opens no comment. Build files carry URLs.
fn strip_comments(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = text.to_string();
    let mut index = 0usize;
    let mut quote: Option<u8> = None;
    while index < bytes.len() {
        let byte = bytes[index];
        match quote {
            Some(mark) => {
                if byte == b'\\' {
                    index += 2;
                    continue;
                }
                if byte == mark {
                    quote = None;
                }
                index += 1;
            }
            None => {
                if byte == b'"' || byte == b'\'' {
                    quote = Some(byte);
                    index += 1;
                } else if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
                    let end = text[index..]
                        .find('\n')
                        .map_or(bytes.len(), |offset| index + offset);
                    blank(&mut out, index, end);
                    index = end;
                } else if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
                    let end = text[index + 2..]
                        .find("*/")
                        .map_or(bytes.len(), |offset| index + 2 + offset + 2);
                    blank(&mut out, index, end);
                    index = end;
                } else {
                    index += 1;
                }
            }
        }
    }
    out
}

/// Overwrite `[start, end)` with spaces, keeping newlines so lines still count.
fn blank(text: &mut String, start: usize, end: usize) {
    // Safe because every replaced byte is ASCII sized, and a multi byte
    // character inside a comment keeps its own bytes replaced one for one.
    let replacement: String = text[start..end]
        .chars()
        .map(|value| if value == '\n' { '\n' } else { ' ' })
        .collect();
    let mut padded = replacement;
    while padded.len() < end - start {
        padded.push(' ');
    }
    text.replace_range(start..end, &padded[..end - start]);
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG: &str = r#"
[versions]
agp = "8.5.0"

[plugins]
android-application = { id = "com.android.application", version.ref = "agp" }
android-library = { id = "com.android.library", version.ref = "agp" }

[libraries]
android-application = { module = "com.android.application:plugin", version.ref = "agp" }
"#;

    fn aliases() -> BTreeMap<String, String> {
        parse_catalog_plugins(CATALOG)
    }

    #[test]
    fn the_catalog_alias_resolves_to_the_application_plugin() {
        let text = "plugins {\n  alias(libs.plugins.android.application)\n}";
        let found = plugin_ids(text, &aliases());
        assert!(found.contains(ANDROID_APPLICATION), "{found:?}");
    }

    #[test]
    fn the_libraries_table_does_not_supply_plugin_aliases() {
        let catalog = "[libraries]\nandroid-application = { id = \"com.android.application\" }\n";
        assert!(parse_catalog_plugins(catalog).is_empty());
    }

    #[test]
    fn every_plugin_declaration_form_is_read() {
        let plain = plugin_ids("plugins { id 'com.android.application' }", &aliases());
        assert!(plain.contains(ANDROID_APPLICATION));
        let kotlin = plugin_ids(r#"plugins { id("com.android.library") }"#, &aliases());
        assert!(kotlin.contains(ANDROID_LIBRARY));
        let legacy = plugin_ids("apply plugin: 'com.android.application'", &aliases());
        assert!(legacy.contains(ANDROID_APPLICATION));
    }

    #[test]
    fn a_plugin_declared_with_apply_false_is_not_applied() {
        let root = r#"plugins {
  alias(libs.plugins.android.application) apply false
  id("com.android.library") version "8.5.0" apply false
}"#;
        assert!(plugin_ids(root, &aliases()).is_empty());
    }

    #[test]
    fn a_commented_out_plugin_is_not_applied() {
        let text =
            "plugins {\n  // id(\"com.android.application\")\n  id(\"com.android.library\")\n}";
        let found = plugin_ids(&strip_comments(text), &aliases());
        assert!(!found.contains(ANDROID_APPLICATION), "{found:?}");
        assert!(found.contains(ANDROID_LIBRARY));
    }

    #[test]
    fn a_url_inside_a_string_does_not_open_a_comment() {
        let text = "maven { url = \"https://example.com/repo\" }\nid(\"com.android.application\")";
        let found = plugin_ids(&strip_comments(text), &aliases());
        assert!(found.contains(ANDROID_APPLICATION), "{found:?}");
    }

    #[test]
    fn test_source_sets_are_named_but_a_flavor_that_starts_with_test_is_not() {
        assert!(is_test_source_set("test"));
        assert!(is_test_source_set("testDebug"));
        assert!(is_test_source_set("androidTest"));
        assert!(is_test_source_set("testFixtures"));
        assert!(is_test_source_set("screenshotTest"));
        assert!(!is_test_source_set("testing"));
        assert!(!is_test_source_set("main"));
        assert!(!is_test_source_set("release"));
    }

    #[test]
    fn a_debuggable_build_type_is_found_by_name_and_by_flag() {
        let text = r#"
android {
  buildTypes {
    release { minifyEnabled true }
    debug { applicationIdSuffix ".debug" }
    staging { debuggable true }
  }
}"#;
        let found = debuggable_build_types(text);
        assert!(found.contains("debug"));
        assert!(found.contains("staging"));
        assert!(!found.contains("release"), "{found:?}");
    }

    #[test]
    fn a_module_with_no_build_types_block_still_has_a_debug_build_type() {
        assert!(debuggable_build_types("android { }").contains("debug"));
    }

    fn sample_project() -> GradleProject {
        let modules = vec![
            ModuleInput {
                build_file: PathBuf::from("build.gradle.kts"),
                text: "plugins { alias(libs.plugins.android.application) apply false }".to_string(),
            },
            ModuleInput {
                build_file: PathBuf::from("app/build.gradle.kts"),
                text: "plugins { alias(libs.plugins.android.application) }\nandroid { buildTypes { release { } debug { } } }".to_string(),
            },
            ModuleInput {
                build_file: PathBuf::from("core/build.gradle.kts"),
                text: "plugins { alias(libs.plugins.android.library) }".to_string(),
            },
        ];
        let paths = vec![
            PathBuf::from("app/src/main/java/App.kt"),
            PathBuf::from("app/src/main/AndroidManifest.xml"),
            PathBuf::from("app/src/debug/AndroidManifest.xml"),
            PathBuf::from("app/src/test/java/AppTest.kt"),
            PathBuf::from("app/src/androidTest/java/AppUiTest.kt"),
            PathBuf::from("core/src/main/java/Core.kt"),
            PathBuf::from("core/src/test/java/CoreTest.kt"),
        ];
        build_gradle_project(&modules, Some(CATALOG), &paths)
    }

    #[test]
    fn the_application_module_is_found_and_the_root_module_is_not_one() {
        let project = sample_project();
        let apps: Vec<_> = project
            .application_modules()
            .map(|module| module.dir.clone())
            .collect();
        assert_eq!(apps, vec![PathBuf::from("app")]);
    }

    #[test]
    fn a_nested_module_owns_its_own_files() {
        let project = sample_project();
        let owner = project.module_for(Path::new("core/src/main/java/Core.kt"));
        assert_eq!(
            owner.map(|module| module.dir.as_path()),
            Some(Path::new("core"))
        );
    }

    #[test]
    fn test_and_debug_files_do_not_ship_but_main_files_do() {
        let project = sample_project();
        assert!(project.ships(Path::new("app/src/main/java/App.kt")));
        assert!(project.ships(Path::new("app/src/main/AndroidManifest.xml")));
        assert!(!project.ships(Path::new("app/src/debug/AndroidManifest.xml")));
        assert!(!project.ships(Path::new("app/src/test/java/AppTest.kt")));
        assert!(!project.ships(Path::new("app/src/androidTest/java/AppUiTest.kt")));
        assert!(!project.ships(Path::new("core/src/test/java/CoreTest.kt")));
    }

    #[test]
    fn a_file_no_module_claims_ships() {
        let project = sample_project();
        assert!(project.ships(Path::new("scripts/tool.kt")));
    }

    #[test]
    fn a_module_named_for_tests_ships_nothing() {
        assert!(is_test_module(Path::new("androidTest-shared")));
        assert!(is_test_module(Path::new("core-test")));
        assert!(is_test_module(Path::new("tests")));
        assert!(is_test_module(Path::new("app-testFixtures")));
        // A name that only holds the letters is still a shipping module.
        assert!(!is_test_module(Path::new("latest")));
        assert!(!is_test_module(Path::new("contest")));
        assert!(!is_test_module(Path::new("app")));
    }

    #[test]
    fn a_test_module_ships_nothing() {
        let modules = vec![ModuleInput {
            build_file: PathBuf::from("benchmark/build.gradle"),
            text: "apply plugin: 'com.android.test'".to_string(),
        }];
        let paths = vec![PathBuf::from("benchmark/src/main/java/Bench.kt")];
        let project = build_gradle_project(&modules, None, &paths);
        assert_eq!(project.modules[0].kind, ModuleKind::Test);
        assert!(!project.ships(Path::new("benchmark/src/main/java/Bench.kt")));
    }
}
