//! End to end checks against the fixture projects.
//!
//! The violating fixtures exercise the detection logic. The compliant
//! fixtures are the false positive calibration set of specification section
//! 11.2: an error severity finding on a compliant project is a false positive.

use peko_check::{engine, PekoConfig, Project};
use peko_rules::{Platform, RuleDatabase, Severity};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn database() -> RuleDatabase {
    RuleDatabase::load_from_dir(root().join("rules")).expect("the rule database must load")
}

fn check(fixture: &str, platform: Platform) -> engine::MechanicalOutcome {
    let db = database();
    let rules = db.active_rules(platform);
    let path = root().join("fixtures").join(fixture);
    // Load the fixture's own .pekorc.json rather than build a fresh config.
    // A fresh one carries no declared fact, so a rule gated on one can never
    // fire and the test reads as a pass on silence. The command had the same
    // fault: --platform threw the project config away.
    let config = PekoConfig::load_or_default(&path, platform).expect("the fixture config loads");
    let project = Project::load(&path, &config).expect("the fixture must load");
    engine::run(&project, &rules, &config, None).expect("the run must finish")
}

fn rule_ids(outcome: &engine::MechanicalOutcome) -> Vec<String> {
    let mut ids: Vec<String> = outcome
        .findings
        .iter()
        .map(|finding| finding.rule_id.to_string())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

#[test]
fn the_ios_project_parses() {
    let config = PekoConfig::new(Platform::Ios);
    let project = Project::load(&root().join("fixtures/ios-compliant"), &config).unwrap();
    assert_eq!(project.bundle_id.as_deref(), Some("com.pekoui.sample"));
    assert_eq!(project.info_plists.len(), 1);
    assert_eq!(project.privacy_manifests.len(), 1);
    assert_eq!(project.entitlements.len(), 1);
    assert_eq!(project.dependencies.len(), 1);
    assert!(project.warnings.is_empty(), "{:?}", project.warnings);
}

#[test]
fn the_android_project_parses() {
    let config = PekoConfig::new(Platform::Android);
    let project = Project::load(&root().join("fixtures/android-violating"), &config).unwrap();
    assert_eq!(project.package_name.as_deref(), Some("dev.peko.bad"));
    assert_eq!(project.android_manifests.len(), 1);
    assert_eq!(project.gradle_settings.len(), 1);
    assert!(project.warnings.is_empty(), "{:?}", project.warnings);
}

#[test]
fn the_violating_ios_project_trips_the_expected_rules() {
    let outcome = check("ios-violating", Platform::Ios);
    let ids = rule_ids(&outcome);
    for expected in [
        "AAPL-API-001",   // UIWebView
        "AAPL-PERM-001",  // the camera purpose string says "Camera"
        "AAPL-PRIV-001",  // UserDefaults with no privacy manifest
        "AAPL-PRIV-010",  // no privacy manifest at all
        "AAPL-PRIV-020",  // the advertising identifier is read
        "AAPL-SEC-001",   // App Transport Security is off
        "AAPL-LEGAL-001", // no export compliance key
    ] {
        assert!(
            ids.contains(&expected.to_string()),
            "{expected} did not fire, found {ids:?}"
        );
    }
}

#[test]
fn the_violating_android_project_trips_the_expected_rules() {
    let outcome = check("android-violating", Platform::Android);
    let ids = rule_ids(&outcome);
    for expected in [
        "GPLAY-PERM-001", // QUERY_ALL_PACKAGES
        "GPLAY-PERM-002", // MANAGE_EXTERNAL_STORAGE
        "GPLAY-PERM-003", // READ_SMS
        "GPLAY-SEC-001",  // cleartext traffic
        "GPLAY-SEC-002",  // automatic backup
        "GPLAY-API-001",  // targetSdk 33
        "GPLAY-PRIV-001", // AdvertisingIdClient
    ] {
        assert!(
            ids.contains(&expected.to_string()),
            "{expected} did not fire, found {ids:?}"
        );
    }
}

#[test]
fn the_compliant_ios_project_raises_no_error() {
    let outcome = check("ios-compliant", Platform::Ios);
    let errors: Vec<String> = outcome
        .findings
        .iter()
        .filter(|finding| finding.severity == Severity::Error)
        .map(|finding| format!("{}: {}", finding.rule_id, finding.message))
        .collect();
    assert!(
        errors.is_empty(),
        "false positives on a compliant project:\n{}",
        errors.join("\n")
    );
}

#[test]
fn the_compliant_android_project_raises_no_error() {
    let outcome = check("android-compliant", Platform::Android);
    let errors: Vec<String> = outcome
        .findings
        .iter()
        .filter(|finding| finding.severity == Severity::Error)
        .map(|finding| format!("{}: {}", finding.rule_id, finding.message))
        .collect();
    assert!(
        errors.is_empty(),
        "false positives on a compliant project:\n{}",
        errors.join("\n")
    );
}

#[test]
fn findings_carry_a_location() {
    let outcome = check("ios-violating", Platform::Ios);
    let uiwebview = outcome
        .findings
        .iter()
        .find(|finding| finding.rule_id.to_string() == "AAPL-API-001")
        .expect("the UIWebView rule must fire");
    let location = uiwebview
        .location
        .as_ref()
        .expect("a source finding needs a location");
    assert_eq!(location.file, PathBuf::from("App/LegacyView.swift"));
    assert!(location.line_start.is_some());
    assert!(location
        .snippet
        .as_deref()
        .unwrap_or_default()
        .contains("UIWebView"));
}

#[test]
fn a_manifest_finding_names_the_line_that_holds_the_value() {
    let outcome = check("android-violating", Platform::Android);
    let finding = outcome
        .findings
        .iter()
        .find(|finding| finding.rule_id.to_string() == "GPLAY-SEC-001")
        .expect("the cleartext traffic rule must fire");
    let location = finding
        .location
        .as_ref()
        .expect("a manifest finding needs a location");
    assert_eq!(
        location.file,
        PathBuf::from("app/src/main/AndroidManifest.xml")
    );
    let snippet = location.snippet.as_deref().unwrap_or_default();
    assert!(
        snippet.contains("usesCleartextTraffic"),
        "the line must hold the attribute, found {snippet:?}"
    );
}

#[test]
fn a_property_list_finding_names_the_key_line() {
    let outcome = check("ios-violating", Platform::Ios);
    let finding = outcome
        .findings
        .iter()
        .find(|finding| finding.rule_id.to_string() == "AAPL-SEC-001")
        .expect("the transport security rule must fire");
    let location = finding
        .location
        .as_ref()
        .expect("a manifest finding needs a location");
    let snippet = location.snippet.as_deref().unwrap_or_default();
    assert!(
        snippet.contains("NSAllowsArbitraryLoads"),
        "the line must hold the key, found {snippet:?}"
    );
}

#[test]
fn a_missing_key_finding_names_the_file_only() {
    let outcome = check("ios-violating", Platform::Ios);
    let finding = outcome
        .findings
        .iter()
        .find(|finding| finding.rule_id.to_string() == "AAPL-LEGAL-001")
        .expect("the export compliance rule must fire");
    let location = finding.location.as_ref().expect("the finding needs a file");
    assert!(
        location.line_start.is_none(),
        "a key that is absent has no line"
    );
}

#[test]
fn the_xcode_targets_are_read() {
    let config = PekoConfig::new(Platform::Ios);
    let project = Project::load(&root().join("fixtures/ios-violating"), &config).unwrap();

    assert_eq!(project.xcode_projects.len(), 1);
    let names: Vec<&str> = project.xcode_projects[0]
        .targets
        .iter()
        .map(|target| target.name.as_str())
        .collect();
    assert_eq!(names, vec!["App", "AppTests"]);
    assert_eq!(project.bundle_id.as_deref(), Some("com.pekoui.bad"));
}

#[test]
fn a_file_that_only_a_test_target_builds_is_not_scanned() {
    let config = PekoConfig::new(Platform::Ios);
    let project = Project::load(&root().join("fixtures/ios-violating"), &config).unwrap();

    let scanned: Vec<String> = project
        .sources
        .iter()
        .map(|file| file.relative().to_string_lossy().into_owned())
        .collect();
    assert!(
        scanned.iter().any(|path| path.contains("LegacyView.swift")),
        "a shipping file must be scanned: {scanned:?}"
    );
    assert!(
        !scanned
            .iter()
            .any(|path| path.contains("HelperTests.swift")),
        "a test only file must not be scanned: {scanned:?}"
    );
}

#[test]
fn a_source_file_maps_back_to_its_target() {
    let config = PekoConfig::new(Platform::Ios);
    let project = Project::load(&root().join("fixtures/ios-violating"), &config).unwrap();

    let owner = project
        .target_for(Path::new("App/LegacyView.swift"))
        .expect("the app target must own the file");
    assert_eq!(owner.name, "App");
    assert_eq!(owner.info_plist(), Some(PathBuf::from("App/Info.plist")));
}

#[test]
fn a_violation_in_a_test_bundle_raises_no_finding() {
    // HelperTests.swift holds a UIWebView reference. The test bundle does not
    // ship, so the checker must stay quiet about it.
    let outcome = check("ios-violating", Platform::Ios);
    let uiwebview: Vec<&str> = outcome
        .findings
        .iter()
        .filter(|finding| finding.rule_id.to_string() == "AAPL-API-001")
        .filter_map(|finding| finding.location.as_ref())
        .map(|location| location.file.to_str().unwrap_or_default())
        .collect();

    assert!(
        uiwebview.iter().all(|path| !path.contains("HelperTests")),
        "a test file must not raise a finding: {uiwebview:?}"
    );
    assert!(
        uiwebview.iter().any(|path| path.contains("LegacyView")),
        "the shipping file must still raise one: {uiwebview:?}"
    );
}

#[test]
fn findings_are_ordered_by_severity() {
    let outcome = check("ios-violating", Platform::Ios);
    let severities: Vec<Severity> = outcome.findings.iter().map(|f| f.severity).collect();
    let mut sorted = severities.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(severities, sorted);
}

#[test]
fn an_override_marks_a_finding_without_removing_it() {
    let db = database();
    let rules = db.active_rules(Platform::Ios);
    let mut config = PekoConfig::new(Platform::Ios);
    config.overrides.push(peko_check::RuleOverride {
        rule_id: "AAPL-SEC-001".parse().unwrap(),
        status: peko_check::OverrideStatus::Acknowledged,
        reason: "The app talks to one legacy host on an internal network.".into(),
        acknowledged_by: Some("preston@pekoui.com".into()),
        acknowledged_at: None,
    });
    let project = Project::load(&root().join("fixtures/ios-violating"), &config).unwrap();
    let outcome = engine::run(&project, &rules, &config, None).unwrap();

    let finding = outcome
        .findings
        .iter()
        .find(|finding| finding.rule_id.to_string() == "AAPL-SEC-001")
        .expect("the finding must stay in the report");
    assert!(finding.overridden);
    assert!(!finding.counts_toward_failure(Severity::Warning));
}

#[test]
fn an_override_cannot_silence_a_rule_that_is_not_overridable() {
    let db = database();
    let rules = db.active_rules(Platform::Ios);
    let mut config = PekoConfig::new(Platform::Ios);
    config.overrides.push(peko_check::RuleOverride {
        rule_id: "AAPL-API-001".parse().unwrap(),
        status: peko_check::OverrideStatus::Acknowledged,
        reason: "We want to ship it anyway.".into(),
        acknowledged_by: None,
        acknowledged_at: None,
    });
    let project = Project::load(&root().join("fixtures/ios-violating"), &config).unwrap();
    let outcome = engine::run(&project, &rules, &config, None).unwrap();

    let finding = outcome
        .findings
        .iter()
        .find(|finding| finding.rule_id.to_string() == "AAPL-API-001")
        .expect("the finding must stay");
    assert!(
        !finding.overridden,
        "a rule that is not overridable must ignore an override"
    );
}

#[test]
fn rules_without_input_are_skipped_not_failed() {
    let outcome = check("ios-compliant", Platform::Ios);
    assert!(outcome.rules_checked > 0);
    // Every mechanical rule lands in exactly one bucket: it ran, its input was
    // absent, or a precondition said it does not apply here.
    assert_eq!(
        outcome.rules_checked + outcome.rules_skipped + outcome.rules_not_applicable,
        database()
            .active_rules(Platform::Ios)
            .iter()
            .filter(|rule| rule.is_mechanical())
            .count()
    );
}

#[test]
fn exclude_paths_remove_a_file_from_the_scan() {
    let db = database();
    let rules = db.active_rules(Platform::Ios);
    let mut config = PekoConfig::new(Platform::Ios);
    config.exclude_paths.push("App/LegacyView.swift".into());
    let project = Project::load(&root().join("fixtures/ios-violating"), &config).unwrap();
    let outcome = engine::run(&project, &rules, &config, None).unwrap();
    let ids = rule_ids(&outcome);
    assert!(
        !ids.contains(&"AAPL-API-001".to_string()),
        "the excluded file was scanned"
    );
}

/// The multi module fixture names its plugins through the version catalog, the
/// form that a plain string search cannot see.
#[test]
fn the_version_catalog_names_the_module_that_ships() {
    let config = PekoConfig::new(Platform::Android);
    let project = Project::load(&root().join("fixtures/android-multi-module"), &config).unwrap();
    let apps: Vec<_> = project
        .gradle_project
        .application_modules()
        .map(|module| module.dir.clone())
        .collect();
    assert_eq!(
        apps,
        vec![PathBuf::from("app")],
        "the app module must be found"
    );

    let flagged: Vec<_> = project
        .gradle_settings
        .iter()
        .filter(|entry| entry.is_application)
        .map(|entry| entry.relative.clone())
        .collect();
    assert_eq!(flagged, vec![PathBuf::from("app/build.gradle.kts")]);
}

#[test]
fn test_and_debug_files_do_not_reach_the_checker() {
    let config = PekoConfig::new(Platform::Android);
    let project = Project::load(&root().join("fixtures/android-multi-module"), &config).unwrap();

    let sources: Vec<_> = project
        .sources
        .iter()
        .map(|file| file.relative().to_path_buf())
        .collect();
    assert!(sources.contains(&PathBuf::from("app/src/main/java/dev/peko/MainActivity.kt")));
    assert!(sources.contains(&PathBuf::from("core/src/main/java/dev/peko/Core.kt")));
    assert!(
        !sources.contains(&PathBuf::from(
            "app/src/androidTest/java/dev/peko/MainActivityTest.kt"
        )),
        "an instrumentation test file must not be checked"
    );
    assert!(
        !sources.contains(&PathBuf::from("core/src/test/java/dev/peko/CoreTest.kt")),
        "a unit test file must not be checked"
    );

    let manifests: Vec<_> = project
        .android_manifests
        .iter()
        .map(|document| document.path().to_path_buf())
        .collect();
    assert!(
        !manifests.contains(&PathBuf::from("app/src/debug/AndroidManifest.xml")),
        "a debug manifest never reaches the store"
    );
}

/// The library manifest sets `allowBackup` to true and the application manifest
/// sets it to false. The merger keeps the application value, so a check of that
/// attribute must read one manifest, not both.
#[test]
fn an_application_attribute_is_read_from_the_shipped_manifest_only() {
    use peko_rules::ManifestFile;
    let config = PekoConfig::new(Platform::Android);
    let project = Project::load(&root().join("fixtures/android-multi-module"), &config).unwrap();

    let attribute = project.manifests_for_key(
        ManifestFile::AndroidManifest,
        "manifest.application.@android:allowBackup",
    );
    assert_eq!(attribute.len(), 1);
    assert_eq!(
        attribute[0].path(),
        Path::new("app/src/main/AndroidManifest.xml")
    );

    // A permission is unioned across every module, so a permission check still
    // reads both manifests.
    let permission = project.manifests_for_key(
        ManifestFile::AndroidManifest,
        "manifest.uses-permission[].@android:name",
    );
    assert_eq!(permission.len(), 2);
}

/// `any_of` holds when one branch holds.
///
/// Every other precondition on a rule is joined with and. A policy that names
/// alternatives needs or: guideline 4.5.4 covers an app that blocks calls,
/// SMS, or MMS, and an app does that three different ways. Verification graded
/// the same rule understated with a gate on one of the three, then overreach
/// with no gate at all. Both readings were right, and the scope sat between
/// them.
#[test]
fn any_of_applies_when_one_branch_holds() {
    use peko_rules::Precondition;

    let db = database();
    let rules = db.active_rules(Platform::Ios);
    let base = rules
        .iter()
        .copied()
        .find(|rule| rule.is_mechanical())
        .expect("a mechanical rule must exist");

    let mut config = PekoConfig::new(Platform::Ios);
    config
        .facts
        .insert("kids_category".to_string(), serde_json::json!(false));
    config
        .facts
        .insert("distributes_in".to_string(), serde_json::json!(["us"]));
    let project = Project::load(&root().join("fixtures/ios-violating"), &config).unwrap();

    let holds = Precondition::FactEquals {
        key: "kids_category".to_string(),
        value: serde_json::json!(false),
    };
    let fails = Precondition::FactEquals {
        key: "kids_category".to_string(),
        value: serde_json::json!(true),
    };

    let mut rule = base.clone();

    // One branch holds, so the rule applies.
    rule.detection.applies_when = vec![Precondition::AnyOf {
        conditions: vec![fails.clone(), holds.clone()],
    }];
    assert_eq!(
        peko_check::engine::rule_applies(&rule, &project, &config),
        peko_check::engine::Applicability::Applies
    );

    // No branch holds, so the rule does not.
    rule.detection.applies_when = vec![Precondition::AnyOf {
        conditions: vec![fails.clone(), fails.clone()],
    }];
    assert_eq!(
        peko_check::engine::rule_applies(&rule, &project, &config),
        peko_check::engine::Applicability::DoesNotApply
    );

    // A branch on an undeclared fact cannot be decided, and no other branch
    // holds, so the answer turns on the fact nobody declared.
    rule.detection.applies_when = vec![Precondition::AnyOf {
        conditions: vec![
            fails,
            Precondition::FactEquals {
                key: "never_declared".to_string(),
                value: serde_json::json!(true),
            },
        ],
    }];
    assert!(matches!(
        peko_check::engine::rule_applies(&rule, &project, &config),
        peko_check::engine::Applicability::Undecided(_)
    ));

    // A branch that holds settles it even when another cannot be decided.
    rule.detection.applies_when = vec![Precondition::AnyOf {
        conditions: vec![
            Precondition::FactEquals {
                key: "never_declared".to_string(),
                value: serde_json::json!(true),
            },
            holds,
        ],
    }];
    assert_eq!(
        peko_check::engine::rule_applies(&rule, &project, &config),
        peko_check::engine::Applicability::Applies
    );
}
