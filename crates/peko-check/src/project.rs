//! The parsed project that the checker runs against.

use crate::config::PekoConfig;
use crate::discovery::{discover, DiscoveredFile};
use crate::error::Result;
use crate::source::SourceFile;
use peko_parse::{
    android, build_gradle_project, config as config_parse, deps, pbxproj, plist_doc, BuildSettings,
    Dependency, Document, FileKind, GradleProject, ModuleInput, XcodeProject, XcodeTarget,
};
use peko_rules::Platform;
use std::path::{Path, PathBuf};

/// A build configuration file and the assignments read out of it.
#[derive(Debug, Clone)]
pub struct ConfigDocument {
    pub relative: PathBuf,
    pub settings: BuildSettings,
    /// True when the file builds the app that ships, not a library module.
    /// An Xcode project file is always true.
    pub is_application: bool,
}

/// Every artifact the mechanical checker reads.
#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub platform: Platform,
    /// Which framework built the app.
    ///
    /// Not a platform. A Flutter app ships an .ipa to Apple and every Apple
    /// rule applies to it. This decides which files hold the dependency graph
    /// and which rules mean something different.
    pub framework: peko_parse::framework::Framework,
    pub name: String,
    pub bundle_id: Option<String>,
    pub package_name: Option<String>,
    pub info_plists: Vec<Document>,
    pub android_manifests: Vec<Document>,
    pub entitlements: Vec<Document>,
    pub privacy_manifests: Vec<Document>,
    pub xcode_settings: Vec<ConfigDocument>,
    /// The parsed Xcode projects. They say which target owns a file, and which
    /// privacy manifest belongs to that target.
    pub xcode_projects: Vec<XcodeProject>,
    pub gradle_settings: Vec<ConfigDocument>,
    /// Manifests and config files, as text, for a rule that names one.
    ///
    /// Separate from `sources` on purpose. A rule scope naming
    /// `**/Info.plist` matched nothing, because the audit globs `sources` and
    /// a manifest never lands there. Putting manifests into `sources` instead
    /// would feed plist and manifest XML to the symbol scans in `derive`,
    /// where a string in a plist would read as a call in code.
    pub readable_documents: Vec<SourceFile>,
    /// The parsed Gradle module graph. It says which module owns a file, which
    /// module builds the shipped APK, and which source sets never ship.
    pub gradle_project: GradleProject,
    /// The index in `android_manifests` of the manifest that the application
    /// module ships. It wins over a library manifest on a merge conflict.
    primary_android_manifest: Option<usize>,
    pub dependencies: Vec<Dependency>,
    pub sources: Vec<SourceFile>,
    /// Files that could not be parsed, and other non-fatal problems.
    pub warnings: Vec<String>,
    /// Files skipped because they are too large.
    pub skipped_large: Vec<PathBuf>,
    /// Facts the checker read from the project itself.
    ///
    /// A fact a person declares wins over one read here, so use
    /// [`Project::fact`] rather than this map.
    pub derived_facts: std::collections::BTreeMap<String, serde_json::Value>,
    /// Facts that came from a registered default rather than from the project
    /// or from `.pekorc.json`. An answer that rests on a guess says so.
    /// Privacy policies that ship in the repository, by path.
    pub policy_documents: Vec<std::path::PathBuf>,
    pub assumed_facts: Vec<String>,
    /// The subset of `assumed_facts` that evidence in the project answered,
    /// rather than a registered default. An inference has a reason behind it.
    pub inferred_facts: Vec<String>,
}

impl Project {
    /// Walk and parse a project directory.
    ///
    /// Without the knowledge base the facts that read dependency flags stay
    /// unknown. Every caller that has the knowledge base must pass it, so
    /// prefer [`Project::load_with_knowledge`].
    pub fn load(root: &Path, config: &PekoConfig) -> Result<Self> {
        Self::load_with_knowledge(root, config, None)
    }

    /// Walk and parse a project directory, and read the dependency flags.
    pub fn load_with_knowledge(
        root: &Path,
        config: &PekoConfig,
        knowledge: Option<&crate::knowledge::KnowledgeBase>,
    ) -> Result<Self> {
        let discovery = discover(root, &config.exclude_paths)?;
        let name = root
            .canonicalize()
            .ok()
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(
                || "project".to_string(),
                |value| value.to_string_lossy().into_owned(),
            );

        let mut project = Self {
            root: root.to_path_buf(),
            platform: config.platform,
            framework: peko_parse::framework::detect(
                &discovery
                    .files
                    .iter()
                    .map(|file| file.path.strip_prefix(root).unwrap_or(&file.path))
                    .collect::<Vec<_>>(),
            ),
            name,
            bundle_id: None,
            package_name: None,
            info_plists: Vec::new(),
            android_manifests: Vec::new(),
            entitlements: Vec::new(),
            privacy_manifests: Vec::new(),
            xcode_settings: Vec::new(),
            xcode_projects: Vec::new(),
            gradle_settings: Vec::new(),
            readable_documents: Vec::new(),
            gradle_project: GradleProject::default(),
            primary_android_manifest: None,
            dependencies: Vec::new(),
            sources: Vec::new(),
            warnings: Vec::new(),
            skipped_large: discovery.skipped_large,
            derived_facts: std::collections::BTreeMap::new(),
            policy_documents: Vec::new(),
            assumed_facts: Vec::new(),
            inferred_facts: Vec::new(),
        };

        for file in &discovery.files {
            project.ingest(file);
        }

        project
            .dependencies
            .sort_by(|a, b| a.package_id.cmp(&b.package_id));
        project
            .dependencies
            .dedup_by(|a, b| a.package_id == b.package_id);
        project.build_module_graph(&discovery.files);
        project.drop_test_only_sources();
        project.drop_non_shipping_android_files();
        project.find_primary_android_manifest();
        project.identify();
        let (facts, assumed, inferred) = crate::derive::derive(&project, Some(config), knowledge);
        project.derived_facts = facts;
        project.assumed_facts = assumed;
        project.inferred_facts = inferred;
        // A fact the developer answered is not an assumption.
        project
            .assumed_facts
            .retain(|name| config.fact(name).is_none());
        Ok(project)
    }

    /// Keep the text of a manifest, so a rule that names one by glob finds it.
    ///
    /// A rule scope naming `**/Info.plist` or `**/AndroidManifest.xml` matched
    /// nothing before this, because the audit globs `sources` and a manifest
    /// never lands there. That is 39 rules each.
    ///
    /// These stay out of `sources` on purpose. The symbol scans in `derive`
    /// read `sources`, and a string inside a plist would read there as a call
    /// in code.
    fn record_readable(&mut self, file: &DiscoveredFile, relative: &Path) {
        if !matches!(
            file.kind,
            FileKind::InfoPlist
                | FileKind::AndroidManifest
                | FileKind::Entitlements
                | FileKind::PrivacyManifest
                | FileKind::BuildGradle
                | FileKind::Pubspec
                | FileKind::PackageJson
                | FileKind::ExpoConfig
                | FileKind::CapacitorConfig
                | FileKind::CsProj
        ) {
            return;
        }
        if let Ok(text) = read_text(&file.path) {
            self.readable_documents
                .push(SourceFile::new(file.path.clone(), relative, text));
        }
    }

    /// Read the dependency list a framework keeps outside the native project.
    ///
    /// Flutter resolves into a cache outside the repository, and npm resolves
    /// into `node_modules`, which the walker excludes. In both cases the
    /// manifest is the only record of what the app links.
    fn ingest_framework_manifest(&mut self, file: &DiscoveredFile, relative: &Path) {
        let text = match read_text(&file.path) {
            Ok(text) => text,
            Err(error) => {
                self.warnings.push(error);
                return;
            }
        };
        match file.kind {
            FileKind::PubspecLock => self
                .dependencies
                .extend(deps::parse_pubspec_lock(relative, &text)),
            FileKind::CsProj => self
                .dependencies
                .extend(deps::parse_csproj(relative, &text)),
            _ => match deps::parse_package_json(relative, &text) {
                Ok(found) => self.dependencies.extend(found),
                Err(error) => self.warnings.push(error.to_string()),
            },
        }
    }

    fn ingest(&mut self, file: &DiscoveredFile) {
        let relative = file.relative.as_path();

        self.record_readable(file, relative);

        match file.kind {
            FileKind::InfoPlist | FileKind::Entitlements | FileKind::PrivacyManifest => {
                match read_bytes(&file.path) {
                    Ok(bytes) => match plist_doc::parse_plist(file.kind, relative, &bytes) {
                        Ok(document) => match file.kind {
                            FileKind::InfoPlist => self.info_plists.push(document),
                            FileKind::Entitlements => self.entitlements.push(document),
                            _ => self.privacy_manifests.push(document),
                        },
                        Err(error) => self.warnings.push(error.to_string()),
                    },
                    Err(error) => self.warnings.push(error),
                }
            }
            // The path is what the checker needs. Nothing reads the prose,
            // so the file is recorded and never opened.
            FileKind::PolicyDocument => self.policy_documents.push(relative.to_path_buf()),
            FileKind::AndroidManifest => match read_text(&file.path) {
                Ok(text) => match android::parse_xml(file.kind, relative, &text) {
                    Ok(document) => self.android_manifests.push(document),
                    Err(error) => self.warnings.push(error.to_string()),
                },
                Err(error) => self.warnings.push(error),
            },
            FileKind::XcodeProject => match read_text(&file.path) {
                Ok(text) => {
                    // The directory that holds the `.xcodeproj` bundle. Every
                    // path inside the project file is relative to it.
                    let root = relative
                        .parent()
                        .and_then(Path::parent)
                        .unwrap_or(Path::new(""));
                    match pbxproj::parse_pbxproj_project(relative, root, &text) {
                        Ok(parsed) => self.xcode_projects.push(parsed),
                        Err(error) => self.warnings.push(error.to_string()),
                    }
                    self.xcode_settings.push(ConfigDocument {
                        relative: relative.to_path_buf(),
                        settings: config_parse::parse_pbxproj(&text),
                        is_application: true,
                    });
                }
                Err(error) => self.warnings.push(error),
            },
            FileKind::BuildGradle => match read_text(&file.path) {
                Ok(text) => {
                    self.dependencies
                        .extend(deps::parse_gradle_dependencies(relative, &text));
                    self.gradle_settings.push(ConfigDocument {
                        relative: relative.to_path_buf(),
                        settings: config_parse::parse_gradle(&text),
                        is_application: config_parse::declares_android_application(&text),
                    });
                }
                Err(error) => self.warnings.push(error),
            },
            FileKind::PodfileLock => match read_text(&file.path) {
                Ok(text) => self
                    .dependencies
                    .extend(deps::parse_podfile_lock(relative, &text)),
                Err(error) => self.warnings.push(error),
            },
            FileKind::PackageResolved => match read_text(&file.path) {
                Ok(text) => match deps::parse_package_resolved(relative, &text) {
                    Ok(found) => self.dependencies.extend(found),
                    Err(error) => self.warnings.push(error.to_string()),
                },
                Err(error) => self.warnings.push(error),
            },
            FileKind::GradleVersionCatalog => match read_text(&file.path) {
                Ok(text) => self
                    .dependencies
                    .extend(deps::parse_version_catalog(relative, &text)),
                Err(error) => self.warnings.push(error),
            },
            FileKind::PubspecLock | FileKind::PackageJson | FileKind::CsProj => {
                self.ingest_framework_manifest(file, relative);
            }
            // Recorded so the framework detector and the rules can see that
            // the file exists. Nothing parses them yet.
            FileKind::Pubspec
            | FileKind::NpmLock
            | FileKind::NuGetLock
            | FileKind::ExpoConfig
            | FileKind::CapacitorConfig => {}
            FileKind::Source => match read_text(&file.path) {
                Ok(text) => self
                    .sources
                    .push(SourceFile::new(file.path.clone(), relative, text)),
                Err(error) => self.warnings.push(error),
            },
        }
    }

    /// Drop source files that only a test target builds.
    ///
    /// A test bundle never reaches the store, so a finding inside one is not a
    /// compliance risk. Before the project file was parsed the checker used a
    /// path glob, which missed a test file with an ordinary name and caught a
    /// shipping file that sat in a folder named `test`.
    fn drop_test_only_sources(&mut self) {
        if self.xcode_projects.is_empty() {
            return;
        }
        let mut test_only: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
        for project in &self.xcode_projects {
            for file in project.test_only_files() {
                test_only.insert(normalize_join(&project.root, &file));
            }
        }
        if test_only.is_empty() {
            return;
        }
        let before = self.sources.len();
        self.sources
            .retain(|file| !test_only.contains(file.relative()));
        let dropped = before - self.sources.len();
        if dropped > 0 {
            self.warnings.push(format!(
                "skipped {dropped} source files that only a test target builds"
            ));
        }
    }

    /// Build the Gradle module graph, and correct which build files build the
    /// shipped APK.
    ///
    /// The first pass looks for the plain string `com.android.application`.
    /// A build file that names the plugin through the version catalog, as
    /// `alias(libs.plugins.android.application)`, does not contain it. The
    /// graph resolves the alias, so a large real app now reports the module
    /// that ships instead of reporting none.
    fn build_module_graph(&mut self, files: &[DiscoveredFile]) {
        let mut inputs: Vec<ModuleInput> = Vec::new();
        let mut catalog: Option<String> = None;
        for file in files {
            match file.kind {
                FileKind::BuildGradle => {
                    if let Ok(text) = read_text(&file.path) {
                        inputs.push(ModuleInput {
                            build_file: file.relative.clone(),
                            text,
                        });
                    }
                }
                FileKind::GradleVersionCatalog if catalog.is_none() => {
                    catalog = read_text(&file.path).ok();
                }
                _ => {}
            }
        }
        if inputs.is_empty() {
            return;
        }
        let paths: Vec<PathBuf> = files.iter().map(|file| file.relative.clone()).collect();
        self.gradle_project = build_gradle_project(&inputs, catalog.as_deref(), &paths);

        for entry in &mut self.gradle_settings {
            let owner = self
                .gradle_project
                .modules
                .iter()
                .find(|module| module.build_file == entry.relative);
            if let Some(module) = owner {
                entry.is_application = module.kind == peko_parse::ModuleKind::Application;
            }
        }
    }

    /// Drop Android files that never reach the store.
    ///
    /// Gradle splits a module into source sets. `src/test` and
    /// `src/androidTest` build only for a test run, and a debug build type
    /// carries settings the release build does not have. A finding in either
    /// is not a compliance risk, and reporting one wastes the reader's time.
    fn drop_non_shipping_android_files(&mut self) {
        if self.gradle_project.is_empty() {
            return;
        }
        let project = &self.gradle_project;
        let before = self.sources.len() + self.android_manifests.len();
        self.sources.retain(|file| project.ships(file.relative()));
        self.android_manifests
            .retain(|document| project.ships(document.path()));
        let dropped = before - self.sources.len() - self.android_manifests.len();
        if dropped > 0 {
            self.warnings.push(format!(
                "skipped {dropped} Android files in source sets that do not ship"
            ));
        }
    }

    /// Record which manifest the application module ships.
    ///
    /// A multi module app holds one manifest per module. Only the manifest in
    /// the `main` source set of the application module is the one the merger
    /// treats as primary.
    fn find_primary_android_manifest(&mut self) {
        let Some(app) = self.gradle_project.application_modules().next() else {
            return;
        };
        let wanted = app.dir.join("src").join("main").join("AndroidManifest.xml");
        self.primary_android_manifest = self
            .android_manifests
            .iter()
            .position(|document| document.path() == wanted);
    }

    /// The value of a fact, from the configuration first and the project next.
    ///
    /// A person sometimes knows the code is dead, and the checker never does,
    /// so a declared answer wins over a derived one.
    pub fn fact<'a>(&'a self, key: &str, config: &'a PekoConfig) -> Option<&'a serde_json::Value> {
        config.fact(key).or_else(|| self.derived_facts.get(key))
    }

    /// The manifests that a check of `key` must read.
    ///
    /// The Android manifest merger unions the permissions of every module, so
    /// a permission check reads every shipping manifest. It does not union the
    /// attributes of the `<application>` element. The application module sets
    /// those, and a library value that it overrides never ships. Reading a
    /// library value as if it shipped reports a setting the store never sees.
    pub fn manifests_for_key(&self, file: peko_rules::ManifestFile, key: &str) -> &[Document] {
        let all = self.manifests(file);
        if file != peko_rules::ManifestFile::AndroidManifest
            || !key.starts_with("manifest.application.@")
        {
            return all;
        }
        match self.primary_android_manifest {
            Some(index) => &all[index..=index],
            None => all,
        }
    }

    /// The privacy manifests that belong to the target that owns `path`.
    ///
    /// A real app ships one manifest per target. A source file in a framework
    /// needs the manifest of that framework, not the manifest of the app. When
    /// no target claims the file, every manifest counts, because a wrong guess
    /// must not turn into a false finding.
    pub fn privacy_manifests_for(&self, path: &Path) -> Vec<&Document> {
        for project in &self.xcode_projects {
            let Ok(inside) = path.strip_prefix(&project.root) else {
                continue;
            };
            let Some(target) = project.target_for(inside) else {
                continue;
            };
            let owned: Vec<&Document> = self
                .privacy_manifests
                .iter()
                .filter(|document| {
                    let Ok(relative) = document.path().strip_prefix(&project.root) else {
                        return false;
                    };
                    target.owns(relative)
                })
                .collect();
            if !owned.is_empty() {
                return owned;
            }
        }
        self.privacy_manifests.iter().collect()
    }

    /// The target that owns a file, across every Xcode project.
    pub fn target_for(&self, path: &Path) -> Option<&XcodeTarget> {
        self.xcode_projects.iter().find_map(|project| {
            let inside = path.strip_prefix(&project.root).ok()?;
            project.target_for(inside)
        })
    }

    /// Read the bundle id and the package name out of the parsed artifacts.
    fn identify(&mut self) {
        self.bundle_id = self
            .xcode_projects
            .iter()
            .flat_map(XcodeProject::app_targets)
            .find_map(|target| target.bundle_id())
            .filter(|value| !value.contains("$("))
            .map(str::to_string)
            .or_else(|| {
                self.info_plists
                    .iter()
                    .find_map(|doc| string_at(doc, "CFBundleIdentifier"))
                    .filter(|value| !value.starts_with("$("))
            })
            .or_else(|| {
                self.xcode_settings.iter().find_map(|config| {
                    config
                        .settings
                        .first("PRODUCT_BUNDLE_IDENTIFIER")
                        .map(|setting| setting.value.clone())
                })
            });

        self.package_name = self
            .android_manifests
            .iter()
            .find_map(|doc| string_at(doc, "manifest.@package"))
            .or_else(|| {
                self.gradle_settings.iter().find_map(|config| {
                    config
                        .settings
                        .first("applicationId")
                        .or_else(|| config.settings.first("namespace"))
                        .map(|setting| setting.value.clone())
                })
            });
    }

    /// The manifest documents for one platform file class.
    pub fn manifests(&self, file: peko_rules::ManifestFile) -> &[Document] {
        match file {
            peko_rules::ManifestFile::InfoPlist => &self.info_plists,
            peko_rules::ManifestFile::AndroidManifest => &self.android_manifests,
            peko_rules::ManifestFile::PrivacyManifest => &self.privacy_manifests,
            peko_rules::ManifestFile::Entitlements => &self.entitlements,
        }
    }

    /// The configuration documents for one file class.
    pub fn configs(&self, file: peko_rules::ConfigFile) -> &[ConfigDocument] {
        match file {
            peko_rules::ConfigFile::XcodeProject => &self.xcode_settings,
            peko_rules::ConfigFile::BuildGradle => &self.gradle_settings,
        }
    }

    /// The count of files the checker read.
    pub fn file_count(&self) -> usize {
        self.info_plists.len()
            + self.android_manifests.len()
            + self.entitlements.len()
            + self.privacy_manifests.len()
            + self.xcode_settings.len()
            + self.gradle_settings.len()
            + self.sources.len()
    }
}

/// Join two relative paths and remove any `..` step.
///
/// An Xcode project inside a subdirectory names files with `../`, for example
/// `WordPress/../Sources/WordPress/Info.plist`. The walker reports the plain
/// path, so the two forms must meet.
fn normalize_join(root: &Path, tail: &Path) -> PathBuf {
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    for component in root.components().chain(tail.components()) {
        match component {
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::CurDir => {}
            other => parts.push(other.as_os_str().to_os_string()),
        }
    }
    parts.iter().collect()
}

fn string_at(document: &Document, path: &str) -> Option<String> {
    document
        .lookup_first(path)
        .ok()
        .flatten()
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn read_bytes(path: &Path) -> std::result::Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))
}

fn read_text(path: &Path) -> std::result::Result<String, String> {
    match std::fs::read(path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Ok(text),
            Err(_) => Err(format!(
                "skipped {}: the file is not UTF-8 text",
                path.display()
            )),
        },
        Err(error) => Err(format!("failed to read {}: {error}", path.display())),
    }
}
