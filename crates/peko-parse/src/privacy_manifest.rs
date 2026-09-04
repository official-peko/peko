//! Reading `PrivacyInfo.xcprivacy` into privacy declarations.
//!
//! Apple requires every SDK to ship this file, and the vendor writes it. That
//! makes it first party evidence about what a package collects, which is
//! exactly what the knowledge base needs and exactly what nobody should invent
//! by hand.
//!
//! The file answers four of the six questions a privacy form asks: the data
//! type, the purpose, whether the data links to an identity, and whether it
//! feeds tracking. It says nothing about the two Play questions, collected or
//! shared and required or optional. This module reports what the file states
//! and leaves the rest unanswered rather than fill it in.

use serde::{Deserialize, Serialize};

/// One collected data type, as the manifest states it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectedDataType {
    /// The short name, with Apple's `NSPrivacyCollectedDataType` prefix gone.
    pub data_type: String,
    pub linked_to_identity: bool,
    pub used_for_tracking: bool,
    /// One entry per purpose the manifest lists, prefix gone.
    pub purposes: Vec<String>,
}

/// What one `PrivacyInfo.xcprivacy` states.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyManifest {
    /// The value of `NSPrivacyTracking`.
    pub tracking: bool,
    pub tracking_domains: Vec<String>,
    pub collected: Vec<CollectedDataType>,
}

/// Drop Apple's long prefix from a constant name.
///
/// `NSPrivacyCollectedDataTypeCrashData` becomes `crash_data`. The knowledge
/// base is read by people and diffed in review, so the short snake case name
/// is what belongs in it.
#[must_use]
pub fn short_name(raw: &str) -> String {
    let trimmed = raw
        .strip_prefix("NSPrivacyCollectedDataTypePurpose")
        .or_else(|| raw.strip_prefix("NSPrivacyCollectedDataType"))
        .unwrap_or(raw);
    // Split on a case change, not on every capital. Apple writes acronyms
    // whole, so DeviceID must read device_id and not device_i_d. A separator
    // belongs before a capital only when the character before it is lower
    // case, or when the character after it is.
    let characters: Vec<char> = trimmed.chars().collect();
    let mut out = String::new();
    for (index, character) in characters.iter().enumerate() {
        if character.is_uppercase() && index > 0 {
            let previous_is_lower = characters[index - 1].is_lowercase();
            let next_is_lower = characters
                .get(index + 1)
                .is_some_and(|next| next.is_lowercase());
            if previous_is_lower || next_is_lower {
                out.push('_');
            }
        }
        out.extend(character.to_lowercase());
    }
    out
}

/// Read a manifest from raw plist bytes.
///
/// # Errors
///
/// Returns an error when the bytes are not a readable plist.
pub fn parse(bytes: &[u8]) -> Result<PrivacyManifest, plist::Error> {
    let root: plist::Value = plist::from_bytes(bytes)?;
    let Some(dictionary) = root.as_dictionary() else {
        return Ok(PrivacyManifest::default());
    };

    let tracking = dictionary
        .get("NSPrivacyTracking")
        .and_then(plist::Value::as_boolean)
        .unwrap_or(false);

    let tracking_domains = dictionary
        .get("NSPrivacyTrackingDomains")
        .and_then(plist::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_string().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let mut collected = Vec::new();
    if let Some(entries) = dictionary
        .get("NSPrivacyCollectedDataTypes")
        .and_then(plist::Value::as_array)
    {
        for entry in entries {
            let Some(entry) = entry.as_dictionary() else {
                continue;
            };
            let Some(raw) = entry
                .get("NSPrivacyCollectedDataType")
                .and_then(plist::Value::as_string)
            else {
                continue;
            };
            let purposes = entry
                .get("NSPrivacyCollectedDataTypePurposes")
                .and_then(plist::Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_string().map(short_name))
                        .collect()
                })
                .unwrap_or_default();
            collected.push(CollectedDataType {
                data_type: short_name(raw),
                // A missing key is false. Apple treats an absent boolean as
                // "no", and reading it as unknown would turn every older
                // manifest into a gap.
                linked_to_identity: entry
                    .get("NSPrivacyCollectedDataTypeLinked")
                    .and_then(plist::Value::as_boolean)
                    .unwrap_or(false),
                used_for_tracking: entry
                    .get("NSPrivacyCollectedDataTypeTracking")
                    .and_then(plist::Value::as_boolean)
                    .unwrap_or(false),
                purposes,
            });
        }
    }

    Ok(PrivacyManifest {
        tracking,
        tracking_domains,
        collected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>NSPrivacyTracking</key><true/>
  <key>NSPrivacyTrackingDomains</key>
  <array><string>ads.example.com</string></array>
  <key>NSPrivacyCollectedDataTypes</key>
  <array>
    <dict>
      <key>NSPrivacyCollectedDataType</key>
      <string>NSPrivacyCollectedDataTypeDeviceID</string>
      <key>NSPrivacyCollectedDataTypeLinked</key><true/>
      <key>NSPrivacyCollectedDataTypeTracking</key><true/>
      <key>NSPrivacyCollectedDataTypePurposes</key>
      <array>
        <string>NSPrivacyCollectedDataTypePurposeThirdPartyAdvertising</string>
      </array>
    </dict>
    <dict>
      <key>NSPrivacyCollectedDataType</key>
      <string>NSPrivacyCollectedDataTypeCrashData</string>
      <key>NSPrivacyCollectedDataTypeLinked</key><false/>
      <key>NSPrivacyCollectedDataTypeTracking</key><false/>
      <key>NSPrivacyCollectedDataTypePurposes</key>
      <array>
        <string>NSPrivacyCollectedDataTypePurposeAppFunctionality</string>
      </array>
    </dict>
  </array>
</dict>
</plist>"#;

    #[test]
    fn reads_every_collected_type() {
        let manifest = parse(SAMPLE.as_bytes()).expect("the sample parses");
        assert!(manifest.tracking);
        assert_eq!(manifest.tracking_domains, vec!["ads.example.com"]);
        assert_eq!(manifest.collected.len(), 2);
    }

    #[test]
    fn keeps_the_tracking_flag_of_each_type_apart() {
        // One tracked type and one untracked type in the same file. A parser
        // that reused the file level NSPrivacyTracking flag for every row
        // would mark crash data as tracking, and that answer goes on a form.
        let manifest = parse(SAMPLE.as_bytes()).expect("parses");
        let device = &manifest.collected[0];
        let crash = &manifest.collected[1];
        assert_eq!(device.data_type, "device_id");
        assert!(device.used_for_tracking);
        assert!(device.linked_to_identity);
        assert_eq!(crash.data_type, "crash_data");
        assert!(!crash.used_for_tracking);
        assert!(!crash.linked_to_identity);
    }

    #[test]
    fn strips_the_purpose_prefix_too() {
        let manifest = parse(SAMPLE.as_bytes()).expect("parses");
        assert_eq!(
            manifest.collected[0].purposes,
            vec!["third_party_advertising"]
        );
        assert_eq!(manifest.collected[1].purposes, vec!["app_functionality"]);
    }

    #[test]
    fn a_manifest_that_collects_nothing_is_not_an_error() {
        // Most utility SDKs ship a manifest with an empty array. That states
        // "collects nothing", which is an answer, not a missing file.
        let empty = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>NSPrivacyTracking</key><false/>
<key>NSPrivacyCollectedDataTypes</key><array/>
</dict></plist>"#;
        let manifest = parse(empty.as_bytes()).expect("parses");
        assert!(!manifest.tracking);
        assert!(manifest.collected.is_empty());
    }

    #[test]
    fn an_entry_without_a_data_type_is_skipped_not_guessed() {
        let broken = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>NSPrivacyCollectedDataTypes</key>
<array><dict><key>NSPrivacyCollectedDataTypeLinked</key><true/></dict></array>
</dict></plist>"#;
        let manifest = parse(broken.as_bytes()).expect("parses");
        assert!(
            manifest.collected.is_empty(),
            "a nameless row reached the output"
        );
    }

    #[test]
    fn a_missing_boolean_reads_as_no() {
        let sparse = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>NSPrivacyCollectedDataTypes</key>
<array><dict>
<key>NSPrivacyCollectedDataType</key><string>NSPrivacyCollectedDataTypeEmailAddress</string>
</dict></array>
</dict></plist>"#;
        let manifest = parse(sparse.as_bytes()).expect("parses");
        assert_eq!(manifest.collected[0].data_type, "email_address");
        assert!(!manifest.collected[0].linked_to_identity);
        assert!(!manifest.collected[0].used_for_tracking);
    }

    #[test]
    fn short_name_splits_every_word() {
        assert_eq!(
            short_name("NSPrivacyCollectedDataTypeDeviceID"),
            "device_id"
        );
        assert_eq!(short_name("NSPrivacyCollectedDataTypeName"), "name");
        assert_eq!(short_name("AlreadyShort"), "already_short");
        // An acronym at the front and one in the middle.
        assert_eq!(short_name("URLSession"), "url_session");
        assert_eq!(short_name("NSPrivacyCollectedDataTypeUserID"), "user_id");
    }

    #[test]
    fn bytes_that_are_not_a_plist_fail_rather_than_read_as_empty() {
        // Silence would look like "this SDK collects nothing", and that is a
        // form answer. A fetch that returned an HTML error page must not
        // become a privacy claim.
        assert!(parse(b"<html>404 not found</html>").is_err());
    }
}
