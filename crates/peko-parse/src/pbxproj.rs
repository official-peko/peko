//! The Xcode project model.
//!
//! A real iOS repository holds many targets: the app, its extensions, its
//! frameworks, and its test bundles. A finding is only meaningful when the
//! checker knows which target a file belongs to.
//!
//! Before this module the checker read the first `Info.plist` it found and the
//! first `PrivacyInfo.xcprivacy` it found. A validation run against published
//! apps produced six false positives from that alone: the code lived in one
//! target and the declaration lived in another.

use crate::error::{ParseError, Result};
use crate::openstep;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// What a target builds.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductType {
    /// The app that ships to the store.
    Application,
    /// A widget, a share sheet, a keyboard, and so on.
    AppExtension,
    Framework,
    StaticLibrary,
    /// A watch app or its extension.
    Watch,
    UnitTest,
    UiTest,
    Bundle,
    Other(String),
}

impl ProductType {
    fn from_identifier(text: &str) -> Self {
        match text {
            "com.apple.product-type.application" => ProductType::Application,
            "com.apple.product-type.app-extension"
            | "com.apple.product-type.extensionkit-extension" => ProductType::AppExtension,
            "com.apple.product-type.framework" => ProductType::Framework,
            "com.apple.product-type.library.static" => ProductType::StaticLibrary,
            "com.apple.product-type.application.watchapp2"
            | "com.apple.product-type.watchkit2-extension" => ProductType::Watch,
            "com.apple.product-type.bundle.unit-test" => ProductType::UnitTest,
            "com.apple.product-type.bundle.ui-testing" => ProductType::UiTest,
            "com.apple.product-type.bundle" => ProductType::Bundle,
            other => ProductType::Other(other.to_string()),
        }
    }

    /// True when the target reaches the store inside the app.
    ///
    /// A test bundle never ships. A finding in a test bundle is not a
    /// compliance risk, and reporting one costs user trust.
    pub fn ships(&self) -> bool {
        matches!(
            self,
            ProductType::Application
                | ProductType::AppExtension
                | ProductType::Framework
                | ProductType::StaticLibrary
                | ProductType::Watch
                | ProductType::Bundle
        )
    }

    /// True when the target is a test bundle.
    pub fn is_test(&self) -> bool {
        matches!(self, ProductType::UnitTest | ProductType::UiTest)
    }
}

/// One target inside an Xcode project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XcodeTarget {
    pub name: String,
    pub product_type: ProductType,
    /// Build settings, taken from the release configuration when there is one.
    pub build_settings: BTreeMap<String, String>,
    /// Source files compiled into this target, relative to the directory that
    /// holds the `.xcodeproj`.
    pub source_files: Vec<PathBuf>,
    /// Resource files copied into this target, relative to the same directory.
    pub resource_files: Vec<PathBuf>,
    /// Folders whose whole contents belong to this target. Xcode 16 and later
    /// use a synchronized folder instead of a file list.
    pub synchronized_folders: Vec<PathBuf>,
}

impl XcodeTarget {
    /// The `Info.plist` of this target, when the build settings name one.
    pub fn info_plist(&self) -> Option<PathBuf> {
        self.setting_path("INFOPLIST_FILE")
    }

    /// The entitlements file of this target, when the build settings name one.
    pub fn entitlements(&self) -> Option<PathBuf> {
        self.setting_path("CODE_SIGN_ENTITLEMENTS")
    }

    pub fn bundle_id(&self) -> Option<&str> {
        self.build_settings
            .get("PRODUCT_BUNDLE_IDENTIFIER")
            .map(String::as_str)
    }

    fn setting_path(&self, key: &str) -> Option<PathBuf> {
        let raw = self.build_settings.get(key)?;
        // A setting that names a variable cannot be resolved without a build.
        if raw.contains('$') {
            return None;
        }
        Some(PathBuf::from(raw.trim_matches('"')))
    }

    /// True when the file belongs to this target.
    ///
    /// A file belongs when the target compiles it, copies it, or holds it
    /// inside a synchronized folder.
    pub fn owns(&self, path: &Path) -> bool {
        self.source_files.iter().any(|file| file == path)
            || self.resource_files.iter().any(|file| file == path)
            || self
                .synchronized_folders
                .iter()
                .any(|folder| path.starts_with(folder))
    }
}

/// A parsed `project.pbxproj`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XcodeProject {
    /// The directory that holds the `.xcodeproj`, relative to the project root.
    pub root: PathBuf,
    pub targets: Vec<XcodeTarget>,
}

impl XcodeProject {
    /// Targets that reach the store.
    pub fn shipping_targets(&self) -> impl Iterator<Item = &XcodeTarget> {
        self.targets
            .iter()
            .filter(|target| target.product_type.ships())
    }

    /// The application targets.
    pub fn app_targets(&self) -> impl Iterator<Item = &XcodeTarget> {
        self.targets
            .iter()
            .filter(|target| target.product_type == ProductType::Application)
    }

    /// Every file that only a test target holds.
    ///
    /// The checker skips these, because a test bundle never ships.
    pub fn test_only_files(&self) -> BTreeSet<PathBuf> {
        let mut shipping: BTreeSet<&PathBuf> = BTreeSet::new();
        for target in self.shipping_targets() {
            shipping.extend(target.source_files.iter());
            shipping.extend(target.resource_files.iter());
        }

        let mut out = BTreeSet::new();
        for target in self.targets.iter().filter(|t| t.product_type.is_test()) {
            for file in target
                .source_files
                .iter()
                .chain(target.resource_files.iter())
            {
                if !shipping.contains(file) {
                    out.insert(file.clone());
                }
            }
        }
        out
    }

    /// The target that owns a file, preferring a shipping target.
    pub fn target_for(&self, path: &Path) -> Option<&XcodeTarget> {
        self.shipping_targets()
            .find(|target| target.owns(path))
            .or_else(|| self.targets.iter().find(|target| target.owns(path)))
    }
}

/// Read a `project.pbxproj`.
///
/// `root` is the directory that holds the `.xcodeproj` bundle. Every path in
/// the result is relative to it.
pub fn parse_pbxproj_project(path: &Path, root: &Path, text: &str) -> Result<XcodeProject> {
    let value = openstep::parse(text).map_err(|source| ParseError::OpenStep {
        path: path.to_path_buf(),
        source,
    })?;

    let objects = value
        .get("objects")
        .and_then(Value::as_object)
        .ok_or_else(|| ParseError::EmptyDocument {
            path: path.to_path_buf(),
        })?;

    let root_id = value
        .get("rootObject")
        .and_then(Value::as_str)
        .unwrap_or("");
    let project = objects.get(root_id);

    // The group tree gives every file reference its path.
    let main_group = project
        .and_then(|entry| entry.get("mainGroup"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut paths: BTreeMap<String, PathBuf> = BTreeMap::new();
    collect_paths(objects, main_group, Path::new(""), &mut paths, 0);

    let target_ids: Vec<&str> = project
        .and_then(|entry| entry.get("targets"))
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut targets = Vec::new();
    for id in target_ids {
        if let Some(target) = read_target(objects, id, &paths) {
            targets.push(target);
        }
    }

    Ok(XcodeProject {
        root: root.to_path_buf(),
        targets,
    })
}

/// Walk the group tree and record the path of every file reference.
///
/// A group carries a path when its `sourceTree` is `<group>`. A group with
/// `SOURCE_ROOT` restarts from the project directory.
fn collect_paths(
    objects: &serde_json::Map<String, Value>,
    id: &str,
    prefix: &Path,
    out: &mut BTreeMap<String, PathBuf>,
    depth: usize,
) {
    // A malformed file can hold a cycle. The depth cap keeps the walk finite.
    if depth > 64 {
        return;
    }
    let Some(node) = objects.get(id) else { return };
    let isa = node.get("isa").and_then(Value::as_str).unwrap_or_default();
    let source_tree = node
        .get("sourceTree")
        .and_then(Value::as_str)
        .unwrap_or("<group>");
    let path = node.get("path").and_then(Value::as_str);

    let here = match (source_tree, path) {
        ("SOURCE_ROOT" | "<absolute>", Some(value)) => PathBuf::from(value),
        (_, Some(value)) => prefix.join(value),
        (_, None) => prefix.to_path_buf(),
    };

    match isa {
        "PBXFileReference" => {
            out.insert(id.to_string(), here);
        }
        "PBXFileSystemSynchronizedRootGroup" => {
            // A synchronized group names a folder, not a file list.
            out.insert(id.to_string(), here);
        }
        _ => {
            let children = node
                .get("children")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for child in children {
                if let Some(child_id) = child.as_str() {
                    collect_paths(objects, child_id, &here, out, depth + 1);
                }
            }
        }
    }
}

fn read_target(
    objects: &serde_json::Map<String, Value>,
    id: &str,
    paths: &BTreeMap<String, PathBuf>,
) -> Option<XcodeTarget> {
    let node = objects.get(id)?;
    if node.get("isa").and_then(Value::as_str)? != "PBXNativeTarget" {
        return None;
    }

    let name = node
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unnamed")
        .to_string();
    let product_type = ProductType::from_identifier(
        node.get("productType")
            .and_then(Value::as_str)
            .unwrap_or(""),
    );

    let mut source_files = Vec::new();
    let mut resource_files = Vec::new();
    for phase_id in node
        .get("buildPhases")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
    {
        let Some(phase) = objects.get(phase_id) else {
            continue;
        };
        let target_list = match phase.get("isa").and_then(Value::as_str) {
            Some("PBXSourcesBuildPhase") => &mut source_files,
            Some("PBXResourcesBuildPhase") => &mut resource_files,
            _ => continue,
        };
        for build_file_id in phase
            .get("files")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str)
        {
            let Some(reference) = objects
                .get(build_file_id)
                .and_then(|file| file.get("fileRef"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            if let Some(path) = paths.get(reference) {
                target_list.push(path.clone());
            }
        }
    }

    let synchronized_folders = node
        .get("fileSystemSynchronizedGroups")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|group_id| paths.get(group_id).cloned())
        .collect();

    Some(XcodeTarget {
        name,
        product_type,
        build_settings: read_build_settings(objects, node),
        source_files,
        resource_files,
        synchronized_folders,
    })
}

/// Read the build settings of a target.
///
/// A target holds one configuration per build, usually Debug and Release. The
/// reader prefers Release, because that is the build that ships.
fn read_build_settings(
    objects: &serde_json::Map<String, Value>,
    target: &Value,
) -> BTreeMap<String, String> {
    let Some(list) = target
        .get("buildConfigurationList")
        .and_then(Value::as_str)
        .and_then(|id| objects.get(id))
    else {
        return BTreeMap::new();
    };

    let configurations: Vec<&str> = list
        .get("buildConfigurations")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let chosen = configurations
        .iter()
        .find(|id| {
            objects
                .get(**id)
                .and_then(|entry| entry.get("name"))
                .and_then(Value::as_str)
                .is_some_and(|name| name.eq_ignore_ascii_case("release"))
        })
        .or(configurations.first());

    let Some(settings) = chosen
        .and_then(|id| objects.get(*id))
        .and_then(|entry| entry.get("buildSettings"))
        .and_then(Value::as_object)
    else {
        return BTreeMap::new();
    };

    settings
        .iter()
        .filter_map(|(key, value)| match value {
            Value::String(text) => Some((key.clone(), text.clone())),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECT: &str = r#"// !$*UTF8*$!
{
    archiveVersion = 1;
    objects = {
        AA01 /* Project object */ = {
            isa = PBXProject;
            mainGroup = BB01;
            targets = ( CC01 /* App */, CC02 /* AppTests */, CC03 /* Widget */, );
        };
        BB01 = {
            isa = PBXGroup;
            children = ( BB02, BB03, BB04, );
            sourceTree = "<group>";
        };
        BB02 = {
            isa = PBXGroup;
            path = App;
            children = ( FF01, FF02, FF03, );
            sourceTree = "<group>";
        };
        BB03 = {
            isa = PBXGroup;
            path = AppTests;
            children = ( FF04, );
            sourceTree = "<group>";
        };
        BB04 = {
            isa = PBXGroup;
            path = Widget;
            children = ( FF05, FF06, );
            sourceTree = "<group>";
        };
        FF01 = { isa = PBXFileReference; path = ContentView.swift; sourceTree = "<group>"; };
        FF02 = { isa = PBXFileReference; path = PrivacyInfo.xcprivacy; sourceTree = "<group>"; };
        FF03 = { isa = PBXFileReference; path = Shared.swift; sourceTree = "<group>"; };
        FF04 = { isa = PBXFileReference; path = AppTests.swift; sourceTree = "<group>"; };
        FF05 = { isa = PBXFileReference; path = WidgetView.swift; sourceTree = "<group>"; };
        FF06 = { isa = PBXFileReference; path = PrivacyInfo.xcprivacy; sourceTree = "<group>"; };
        GG01 = { isa = PBXBuildFile; fileRef = FF01; };
        GG02 = { isa = PBXBuildFile; fileRef = FF02; };
        GG03 = { isa = PBXBuildFile; fileRef = FF03; };
        GG04 = { isa = PBXBuildFile; fileRef = FF04; };
        GG05 = { isa = PBXBuildFile; fileRef = FF05; };
        GG06 = { isa = PBXBuildFile; fileRef = FF06; };
        SS01 = { isa = PBXSourcesBuildPhase; files = ( GG01, GG03, ); };
        RR01 = { isa = PBXResourcesBuildPhase; files = ( GG02, ); };
        SS02 = { isa = PBXSourcesBuildPhase; files = ( GG04, GG03, ); };
        SS03 = { isa = PBXSourcesBuildPhase; files = ( GG05, ); };
        RR03 = { isa = PBXResourcesBuildPhase; files = ( GG06, ); };
        CC01 = {
            isa = PBXNativeTarget;
            name = App;
            productType = "com.apple.product-type.application";
            buildPhases = ( SS01, RR01, );
            buildConfigurationList = LL01;
        };
        CC02 = {
            isa = PBXNativeTarget;
            name = AppTests;
            productType = "com.apple.product-type.bundle.unit-test";
            buildPhases = ( SS02, );
            buildConfigurationList = LL02;
        };
        CC03 = {
            isa = PBXNativeTarget;
            name = Widget;
            productType = "com.apple.product-type.app-extension";
            buildPhases = ( SS03, RR03, );
            buildConfigurationList = LL03;
        };
        LL01 = { isa = XCConfigurationList; buildConfigurations = ( XX01, XX02, ); };
        LL02 = { isa = XCConfigurationList; buildConfigurations = ( XX03, ); };
        LL03 = { isa = XCConfigurationList; buildConfigurations = ( XX04, ); };
        XX01 = {
            isa = XCBuildConfiguration;
            name = Debug;
            buildSettings = { PRODUCT_BUNDLE_IDENTIFIER = com.example.debug; };
        };
        XX02 = {
            isa = XCBuildConfiguration;
            name = Release;
            buildSettings = {
                PRODUCT_BUNDLE_IDENTIFIER = com.example.app;
                INFOPLIST_FILE = "App/Info.plist";
                CODE_SIGN_ENTITLEMENTS = "App/App.entitlements";
                IPHONEOS_DEPLOYMENT_TARGET = 16.0;
            };
        };
        XX03 = { isa = XCBuildConfiguration; name = Release; buildSettings = { }; };
        XX04 = {
            isa = XCBuildConfiguration;
            name = Release;
            buildSettings = { PRODUCT_BUNDLE_IDENTIFIER = com.example.app.widget; };
        };
    };
    rootObject = AA01;
}"#;

    fn project() -> XcodeProject {
        parse_pbxproj_project(
            Path::new("App.xcodeproj/project.pbxproj"),
            Path::new(""),
            PROJECT,
        )
        .unwrap()
    }

    #[test]
    fn every_target_reads() {
        let project = project();
        let names: Vec<&str> = project.targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["App", "AppTests", "Widget"]);
    }

    #[test]
    fn a_product_type_says_whether_the_target_ships() {
        let project = project();
        assert_eq!(project.shipping_targets().count(), 2);
        assert_eq!(project.app_targets().count(), 1);
        assert!(project.targets[1].product_type.is_test());
    }

    #[test]
    fn a_file_resolves_to_its_full_path() {
        let project = project();
        let app = &project.targets[0];
        assert!(app
            .source_files
            .contains(&PathBuf::from("App/ContentView.swift")));
        assert!(app
            .resource_files
            .contains(&PathBuf::from("App/PrivacyInfo.xcprivacy")));
    }

    #[test]
    fn build_settings_come_from_the_release_configuration() {
        let project = project();
        let app = &project.targets[0];
        assert_eq!(app.bundle_id(), Some("com.example.app"));
        assert_eq!(app.info_plist(), Some(PathBuf::from("App/Info.plist")));
        assert_eq!(
            app.entitlements(),
            Some(PathBuf::from("App/App.entitlements"))
        );
        assert_eq!(
            app.build_settings
                .get("IPHONEOS_DEPLOYMENT_TARGET")
                .map(String::as_str),
            Some("16.0")
        );
    }

    #[test]
    fn a_file_maps_back_to_its_target() {
        let project = project();
        let owner = project
            .target_for(Path::new("Widget/WidgetView.swift"))
            .unwrap();
        assert_eq!(owner.name, "Widget");

        // A file in two targets resolves to the one that ships.
        let shared = project.target_for(Path::new("App/Shared.swift")).unwrap();
        assert_eq!(shared.name, "App");
    }

    #[test]
    fn a_test_only_file_is_named() {
        let project = project();
        let test_only = project.test_only_files();
        assert!(test_only.contains(&PathBuf::from("AppTests/AppTests.swift")));
        // Shared.swift builds into the app as well, so it is not test only.
        assert!(!test_only.contains(&PathBuf::from("App/Shared.swift")));
    }

    #[test]
    fn each_target_keeps_its_own_privacy_manifest() {
        let project = project();
        let app = project
            .target_for(Path::new("App/ContentView.swift"))
            .unwrap();
        assert!(app
            .resource_files
            .contains(&PathBuf::from("App/PrivacyInfo.xcprivacy")));

        let widget = project
            .target_for(Path::new("Widget/WidgetView.swift"))
            .unwrap();
        assert!(widget
            .resource_files
            .contains(&PathBuf::from("Widget/PrivacyInfo.xcprivacy")));
    }

    #[test]
    fn a_setting_that_names_a_variable_yields_no_path() {
        let mut target = project().targets[0].clone();
        target
            .build_settings
            .insert("INFOPLIST_FILE".into(), "$(SRCROOT)/App/Info.plist".into());
        assert_eq!(target.info_plist(), None);
    }
}
