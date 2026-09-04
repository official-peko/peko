//! Reading a compiled app bundle: `.ipa`, `.apk`, and `.aab`.
//!
//! Source analysis reads what a developer wrote. A bundle holds what actually
//! ships, and the two differ in ways that cause rejections:
//!
//! - A framework linked transitively appears in no lockfile, and its privacy
//!   manifest is the only record that it collects anything.
//! - A privacy manifest present in the source tree is not always copied into
//!   the built product.
//! - The shipped ABIs decide whether the upload meets the 64 bit requirement,
//!   and nothing in the source says which ones the build produced.
//!
//! All three formats are zip archives, so one reader serves them.
//!
//! This module reports what the bundle contains. It draws no conclusion about
//! whether the contents are correct, because that is a rule's job and a rule
//! cites a policy section for it.

use crate::error::{ParseError, Result};
use crate::privacy_manifest::{self, PrivacyManifest};
use serde::{Deserialize, Serialize};
use std::io::{Read as _, Seek};
use std::path::Path;

/// Which store a bundle targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BundleKind {
    /// An iOS app archive.
    Ipa,
    /// An Android package.
    Apk,
    /// An Android App Bundle.
    Aab,
}

impl BundleKind {
    /// Guess the kind from a file name.
    pub fn from_path(path: &Path) -> Option<Self> {
        match path
            .extension()
            .and_then(|value| value.to_str())?
            .to_ascii_lowercase()
            .as_str()
        {
            "ipa" => Some(Self::Ipa),
            "apk" => Some(Self::Apk),
            "aab" => Some(Self::Aab),
            _ => None,
        }
    }
}

/// One framework or library bundled inside an app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedFramework {
    /// The framework name, without the `.framework` suffix.
    pub name: String,
    /// The path inside the archive.
    pub path: String,
    /// The framework's own privacy manifest, when it ships one.
    ///
    /// `None` means the framework carries no manifest. Apple requires one from
    /// the SDKs on its list, so an absence here is worth reporting. It is not
    /// proof of a violation, because the list does not cover every framework.
    pub privacy_manifest: Option<PrivacyManifest>,
}

/// What a bundle holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bundle {
    /// The app's own `Info.plist`, as raw key and value pairs.
    ///
    /// Only string, integer, and boolean values survive. A nested dictionary
    /// stays out, because the rules that read this ask about flat keys.
    pub info_plist: std::collections::BTreeMap<String, String>,
    /// The app level privacy manifest, when the build copied one in.
    pub privacy_manifest: Option<PrivacyManifest>,
    /// Frameworks the build embedded.
    pub frameworks: Vec<EmbeddedFramework>,
    /// The native ABIs the bundle ships, read from `lib/<abi>/` paths.
    pub abis: Vec<String>,
    /// Every entry path, so a rule can ask a question this struct does not.
    pub entries: Vec<String>,
    /// True when the archive holds an `embedded.mobileprovision`.
    pub has_provisioning_profile: bool,
}

impl Bundle {
    /// Frameworks that ship no privacy manifest of their own.
    ///
    /// Apple requires one from the SDKs on its list. A framework here is a
    /// question for a person, not a finding on its own.
    pub fn frameworks_without_a_manifest(&self) -> impl Iterator<Item = &EmbeddedFramework> {
        self.frameworks
            .iter()
            .filter(|framework| framework.privacy_manifest.is_none())
    }

    /// Whether the bundle ships a 64 bit ABI.
    ///
    /// Returns `None` when the bundle ships no native code at all, which is
    /// not the same as failing the requirement. A pure Kotlin app has no
    /// `lib/` directory and meets the rule by having nothing to port.
    pub fn has_64_bit_abi(&self) -> Option<bool> {
        if self.abis.is_empty() {
            return None;
        }
        Some(
            self.abis
                .iter()
                .any(|abi| abi == "arm64-v8a" || abi == "x86_64"),
        )
    }
}

/// The most bytes one entry may expand to.
///
/// A zip entry states its own uncompressed size, and a crafted archive states
/// a small one and delivers gigabytes. Reading with a cap means a malicious
/// bundle costs a bounded amount of memory instead of the process.
const MAX_ENTRY_BYTES: u64 = 16 * 1024 * 1024;

/// The most entries one bundle may hold.
///
/// A real app archive holds thousands. A million tiny entries is an attack on
/// whoever unpacks it.
const MAX_ENTRIES: usize = 200_000;

fn read_capped<R: std::io::Read>(reader: &mut R, limit: u64) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    reader
        .take(limit)
        .read_to_end(&mut buffer)
        .map_err(|source| ParseError::Io {
            path: std::path::PathBuf::from("<zip entry>"),
            source,
        })?;
    Ok(buffer)
}

/// Flatten the scalar values of a plist into strings.
fn flatten_plist(bytes: &[u8]) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let Ok(value) = plist::from_bytes::<plist::Value>(bytes) else {
        return out;
    };
    let Some(dictionary) = value.as_dictionary() else {
        return out;
    };
    for (key, value) in dictionary {
        let rendered = match value {
            plist::Value::String(text) => text.clone(),
            plist::Value::Integer(number) => number.to_string(),
            plist::Value::Boolean(flag) => flag.to_string(),
            plist::Value::Real(number) => number.to_string(),
            _ => continue,
        };
        out.insert(key.clone(), rendered);
    }
    out
}

/// The framework name for a path such as `Payload/A.app/Frameworks/B.framework/B`.
fn framework_name(path: &str) -> Option<String> {
    path.split('/')
        .find(|part| part.ends_with(".framework"))
        .map(|part| part.trim_end_matches(".framework").to_string())
}

/// The ABI directory for a path such as `lib/arm64-v8a/libfoo.so`.
///
/// Both `lib/` and the App Bundle's `base/lib/` layout are read, because an
/// `.aab` nests every module under its own directory.
fn abi_name(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    let index = parts.iter().position(|part| *part == "lib")?;
    let abi = parts.get(index + 1)?;
    if parts.len() > index + 2 && !abi.is_empty() {
        Some((*abi).to_string())
    } else {
        None
    }
}

/// Read a bundle from an open archive.
///
/// # Errors
///
/// Returns an error when the archive is not readable as a zip.
pub fn read<R: std::io::Read + Seek>(reader: R, path: &Path) -> Result<Bundle> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|error| ParseError::Bundle {
        path: path.to_path_buf(),
        reason: format!("not a readable archive: {error}"),
    })?;

    if archive.len() > MAX_ENTRIES {
        return Err(ParseError::Bundle {
            path: path.to_path_buf(),
            reason: format!("{} entries, over the {MAX_ENTRIES} limit", archive.len()),
        });
    }

    let mut bundle = Bundle::default();
    let mut abis = std::collections::BTreeSet::new();
    let mut framework_manifests: std::collections::BTreeMap<String, PrivacyManifest> =
        std::collections::BTreeMap::new();
    let mut framework_paths: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();

    let names: Vec<String> = (0..archive.len())
        .filter_map(|index| {
            archive
                .by_index(index)
                .ok()
                .map(|entry| entry.name().to_string())
        })
        .collect();

    for name in &names {
        bundle.entries.push(name.clone());
        if let Some(abi) = abi_name(name) {
            abis.insert(abi);
        }
        if name.ends_with("embedded.mobileprovision") {
            bundle.has_provisioning_profile = true;
        }
        if let Some(framework) = framework_name(name) {
            framework_paths
                .entry(framework)
                .or_insert_with(|| name.clone());
        }
    }

    for name in &names {
        let is_info = name.ends_with(".app/Info.plist");
        let is_privacy = name.ends_with("PrivacyInfo.xcprivacy");
        if !is_info && !is_privacy {
            continue;
        }
        let Ok(mut entry) = archive.by_name(name) else {
            continue;
        };
        let Ok(bytes) = read_capped(&mut entry, MAX_ENTRY_BYTES) else {
            continue;
        };
        if is_info {
            // The app's own Info.plist sits directly under the .app. A
            // framework carries one too, and taking the wrong one reports the
            // framework's bundle id as the app's.
            if name.matches(".app/").count() == 1 && !name.contains(".framework") {
                bundle.info_plist = flatten_plist(&bytes);
            }
            continue;
        }
        let Ok(manifest) = privacy_manifest::parse(&bytes) else {
            continue;
        };
        match framework_name(name) {
            Some(framework) => {
                framework_manifests.insert(framework, manifest);
            }
            None => bundle.privacy_manifest = Some(manifest),
        }
    }

    bundle.abis = abis.into_iter().collect();
    bundle.frameworks = framework_paths
        .into_iter()
        .map(|(name, path)| EmbeddedFramework {
            privacy_manifest: framework_manifests.get(&name).cloned(),
            name,
            path,
        })
        .collect();
    Ok(bundle)
}

/// Read a bundle from a file.
///
/// # Errors
///
/// Returns an error when the file cannot be opened or read as a zip.
pub fn read_file(path: &Path) -> Result<Bundle> {
    let file = std::fs::File::open(path).map_err(|source| ParseError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    read(std::io::BufReader::new(file), path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write as _};

    const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>NSPrivacyTracking</key><true/>
<key>NSPrivacyCollectedDataTypes</key>
<array><dict>
<key>NSPrivacyCollectedDataType</key><string>NSPrivacyCollectedDataTypeDeviceID</string>
<key>NSPrivacyCollectedDataTypeLinked</key><true/>
<key>NSPrivacyCollectedDataTypeTracking</key><true/>
</dict></array>
</dict></plist>"#;

    const INFO: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.example.app</string>
<key>MinimumOSVersion</key><string>15.0</string>
<key>ITSAppUsesNonExemptEncryption</key><false/>
</dict></plist>"#;

    fn archive(files: &[(&str, &str)]) -> Cursor<Vec<u8>> {
        let mut buffer = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buffer));
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, body) in files {
                writer.start_file(*name, options).expect("start");
                writer.write_all(body.as_bytes()).expect("write");
            }
            writer.finish().expect("finish");
        }
        Cursor::new(buffer)
    }

    fn ipa(files: &[(&str, &str)]) -> Bundle {
        read(archive(files), Path::new("test.ipa")).expect("the archive reads")
    }

    #[test]
    fn the_kind_comes_from_the_extension() {
        assert_eq!(
            BundleKind::from_path(Path::new("a.ipa")),
            Some(BundleKind::Ipa)
        );
        assert_eq!(
            BundleKind::from_path(Path::new("a.APK")),
            Some(BundleKind::Apk)
        );
        assert_eq!(
            BundleKind::from_path(Path::new("a.aab")),
            Some(BundleKind::Aab)
        );
        assert_eq!(BundleKind::from_path(Path::new("a.zip")), None);
    }

    #[test]
    fn the_app_info_plist_is_read() {
        let bundle = ipa(&[("Payload/My.app/Info.plist", INFO)]);
        assert_eq!(
            bundle
                .info_plist
                .get("CFBundleIdentifier")
                .map(String::as_str),
            Some("com.example.app")
        );
        assert_eq!(
            bundle
                .info_plist
                .get("ITSAppUsesNonExemptEncryption")
                .map(String::as_str),
            Some("false")
        );
    }

    #[test]
    fn a_nested_watch_app_never_replaces_the_main_app() {
        // A watch app sits at Payload/My.app/Watch/MyWatch.app/Info.plist, so
        // its path ends with .app/Info.plist exactly like the main app's.
        // Taking it reports the watch bundle id as the app's, and every rule
        // that reads the bundle id then checks the wrong target.
        //
        // The watch entry comes second here on purpose. A reader with no
        // guard keeps the last one it sees, so an ordering that put the main
        // app last would pass either way.
        let bundle = ipa(&[
            ("Payload/My.app/Info.plist", INFO),
            (
                "Payload/My.app/Watch/MyWatch.app/Info.plist",
                r#"<?xml version="1.0"?><plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.example.app.watchkitapp</string></dict></plist>"#,
            ),
        ]);
        assert_eq!(
            bundle
                .info_plist
                .get("CFBundleIdentifier")
                .map(String::as_str),
            Some("com.example.app")
        );
    }

    #[test]
    fn an_app_privacy_manifest_is_read_from_the_built_product() {
        // A manifest in the source tree is not always copied into the build.
        // The bundle is the only place that shows what actually shipped.
        let bundle = ipa(&[
            ("Payload/My.app/Info.plist", INFO),
            ("Payload/My.app/PrivacyInfo.xcprivacy", MANIFEST),
        ]);
        let manifest = bundle.privacy_manifest.expect("the app manifest shipped");
        assert!(manifest.tracking);
        assert_eq!(manifest.collected[0].data_type, "device_id");
    }

    #[test]
    fn a_missing_app_manifest_reads_as_absent_not_as_empty() {
        let bundle = ipa(&[("Payload/My.app/Info.plist", INFO)]);
        assert!(
            bundle.privacy_manifest.is_none(),
            "an absent manifest read as one that collects nothing"
        );
    }

    #[test]
    fn a_framework_manifest_attaches_to_its_own_framework() {
        let bundle = ipa(&[
            ("Payload/My.app/Info.plist", INFO),
            (
                "Payload/My.app/Frameworks/Tracker.framework/Tracker",
                "binary",
            ),
            (
                "Payload/My.app/Frameworks/Tracker.framework/PrivacyInfo.xcprivacy",
                MANIFEST,
            ),
        ]);
        assert_eq!(bundle.frameworks.len(), 1);
        let framework = &bundle.frameworks[0];
        assert_eq!(framework.name, "Tracker");
        let manifest = framework
            .privacy_manifest
            .as_ref()
            .expect("the framework ships one");
        assert!(manifest.collected[0].used_for_tracking);
        assert!(
            bundle.privacy_manifest.is_none(),
            "a framework manifest was read as the app's own"
        );
    }

    #[test]
    fn a_framework_without_a_manifest_is_listed() {
        // This is the case the whole module exists for. A framework linked
        // transitively appears in no lockfile, and nothing else records that
        // it shipped.
        let bundle = ipa(&[
            ("Payload/My.app/Info.plist", INFO),
            ("Payload/My.app/Frameworks/Quiet.framework/Quiet", "binary"),
            ("Payload/My.app/Frameworks/Loud.framework/Loud", "binary"),
            (
                "Payload/My.app/Frameworks/Loud.framework/PrivacyInfo.xcprivacy",
                MANIFEST,
            ),
        ]);
        let missing: Vec<&str> = bundle
            .frameworks_without_a_manifest()
            .map(|framework| framework.name.as_str())
            .collect();
        assert_eq!(missing, vec!["Quiet"]);
    }

    #[test]
    fn a_provisioning_profile_is_noticed() {
        let bundle = ipa(&[
            ("Payload/My.app/Info.plist", INFO),
            ("Payload/My.app/embedded.mobileprovision", "der bytes"),
        ]);
        assert!(bundle.has_provisioning_profile);
    }

    #[test]
    fn the_shipped_abis_come_from_the_lib_paths() {
        let bundle = read(
            archive(&[
                ("lib/arm64-v8a/libmain.so", "elf"),
                ("lib/armeabi-v7a/libmain.so", "elf"),
                ("classes.dex", "dex"),
            ]),
            Path::new("app.apk"),
        )
        .expect("reads");
        assert_eq!(bundle.abis, vec!["arm64-v8a", "armeabi-v7a"]);
        assert_eq!(bundle.has_64_bit_abi(), Some(true));
    }

    #[test]
    fn a_32_bit_only_bundle_reports_false_not_none() {
        // Google refuses an upload without a 64 bit ABI. The two answers must
        // not collapse, because one is a rejection and one is fine.
        let bundle = read(
            archive(&[("lib/armeabi-v7a/libmain.so", "elf")]),
            Path::new("app.apk"),
        )
        .expect("reads");
        assert_eq!(bundle.has_64_bit_abi(), Some(false));
    }

    #[test]
    fn a_bundle_with_no_native_code_reports_none_not_false() {
        // A pure Kotlin app ships no lib directory. It meets the requirement
        // by having nothing to port, and reporting false would be a finding
        // against an app that is fine.
        let bundle = read(archive(&[("classes.dex", "dex")]), Path::new("app.apk")).expect("reads");
        assert_eq!(bundle.has_64_bit_abi(), None);
    }

    #[test]
    fn an_app_bundle_nests_its_libraries_under_a_module() {
        // An .aab puts every module in its own directory, so base/lib/... is
        // the normal shape and a reader that only knows lib/ finds nothing.
        let bundle = read(
            archive(&[("base/lib/arm64-v8a/libmain.so", "elf")]),
            Path::new("app.aab"),
        )
        .expect("reads");
        assert_eq!(bundle.abis, vec!["arm64-v8a"]);
    }

    #[test]
    fn bytes_that_are_not_an_archive_fail_rather_than_read_as_empty() {
        // An empty bundle would report no frameworks and no manifests, which
        // reads as a clean app.
        let result = read(Cursor::new(b"not a zip".to_vec()), Path::new("broken.ipa"));
        assert!(result.is_err());
    }

    #[test]
    fn every_entry_is_listed_for_a_rule_to_read() {
        let bundle = ipa(&[
            ("Payload/My.app/Info.plist", INFO),
            ("Payload/My.app/Assets.car", "assets"),
        ]);
        assert!(bundle
            .entries
            .iter()
            .any(|entry| entry.ends_with("Assets.car")));
        assert_eq!(bundle.entries.len(), 2);
    }
}
