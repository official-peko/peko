//! Turning an SDK's own privacy manifest into a knowledge base entry.
//!
//! The manifest is first party evidence. Apple requires it, the vendor writes
//! it, and it states four of the six answers a privacy form needs. This module
//! carries those four across and leaves the other two unanswered.
//!
//! The two it leaves alone are the Play questions: collected or shared, and
//! required or optional. An Apple manifest says nothing about either. Filling
//! them from an Apple file would be a guess wearing the clothes of evidence,
//! and the provenance field would then say a lie.

use crate::knowledge::{DeclarationsState, PackageEntry, PrivacyDeclaration, Provenance};
use chrono::{DateTime, Utc};
use peko_parse::privacy_manifest::PrivacyManifest;
use peko_rules::Ecosystem;

/// Build the declarations one manifest states.
///
/// One declaration per data type and purpose pair, because a form asks the
/// purpose per data type. A data type with no purpose listed still produces a
/// row, since the collection itself is the fact that matters.
#[must_use]
pub fn declarations_from(manifest: &PrivacyManifest, source_url: &str) -> Vec<PrivacyDeclaration> {
    let mut out = Vec::new();
    for collected in &manifest.collected {
        let purposes: Vec<String> = if collected.purposes.is_empty() {
            vec!["unstated".to_string()]
        } else {
            collected.purposes.clone()
        };
        for purpose in purposes {
            out.push(PrivacyDeclaration {
                data_type: collected.data_type.clone(),
                purpose,
                linked_to_identity: collected.linked_to_identity,
                used_for_tracking: Some(collected.used_for_tracking),
                // An Apple manifest answers neither Play question. Leaving
                // them None keeps them visible as gaps.
                collection: None,
                optionality: None,
                provenance: Some(Provenance::ApplePrivacyManifest),
                source_url: Some(source_url.to_string()),
            });
        }
    }
    out
}

/// Build a whole entry for one package.
#[must_use]
pub fn entry_from(
    package_id: &str,
    package_name: &str,
    ecosystem: Ecosystem,
    manifest: &PrivacyManifest,
    source_url: &str,
    at: DateTime<Utc>,
) -> PackageEntry {
    PackageEntry {
        package_id: package_id.to_string(),
        package_name: package_name.to_string(),
        ecosystem,
        compliance_flags: Vec::new(),
        required_privacy_declarations: declarations_from(manifest, source_url),
        declarations_state: DeclarationsState::Curated,
        last_updated: at,
    }
}

/// Record that a package's repository publishes no privacy manifest.
///
/// The empty declaration list here is an absence, not an answer, and the state
/// says so. Writing this as `Curated` would tell a developer the package
/// collects nothing, which nobody checked.
#[must_use]
pub fn entry_without_manifest(
    package_id: &str,
    package_name: &str,
    ecosystem: Ecosystem,
    at: DateTime<Utc>,
) -> PackageEntry {
    PackageEntry {
        package_id: package_id.to_string(),
        package_name: package_name.to_string(),
        ecosystem,
        compliance_flags: Vec::new(),
        required_privacy_declarations: Vec::new(),
        declarations_state: DeclarationsState::NoManifestPublished,
        last_updated: at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use peko_parse::privacy_manifest::CollectedDataType;

    fn manifest(collected: Vec<CollectedDataType>) -> PrivacyManifest {
        PrivacyManifest {
            tracking: false,
            tracking_domains: Vec::new(),
            collected,
        }
    }

    fn collected(
        data_type: &str,
        linked: bool,
        tracking: bool,
        purposes: &[&str],
    ) -> CollectedDataType {
        CollectedDataType {
            data_type: data_type.to_string(),
            linked_to_identity: linked,
            used_for_tracking: tracking,
            purposes: purposes.iter().map(|value| (*value).to_string()).collect(),
        }
    }

    #[test]
    fn the_apple_answers_come_across_and_the_play_answers_do_not() {
        // This is the whole contract of the module. An Apple manifest answers
        // the iOS label. It says nothing about the Play form, and inventing
        // those two answers would be worse than leaving them blank.
        let source = "https://github.com/vendor/sdk/blob/main/PrivacyInfo.xcprivacy";
        let declarations = declarations_from(
            &manifest(vec![collected("device_id", true, true, &["analytics"])]),
            source,
        );
        assert_eq!(declarations.len(), 1);
        let one = &declarations[0];
        assert_eq!(one.data_type, "device_id");
        assert!(one.linked_to_identity);
        assert_eq!(one.used_for_tracking, Some(true));
        assert_eq!(
            one.collection, None,
            "an Apple file answered a Play question"
        );
        assert_eq!(
            one.optionality, None,
            "an Apple file answered a Play question"
        );
        assert_eq!(one.provenance, Some(Provenance::ApplePrivacyManifest));
        assert_eq!(one.source_url.as_deref(), Some(source));
    }

    #[test]
    fn one_row_per_purpose() {
        // A form asks the purpose per data type, so two purposes are two
        // answers, not one row with a joined string nobody can read back.
        let declarations = declarations_from(
            &manifest(vec![collected(
                "email_address",
                true,
                false,
                &["app_functionality", "analytics"],
            )]),
            "url",
        );
        assert_eq!(declarations.len(), 2);
        assert_eq!(declarations[0].purpose, "app_functionality");
        assert_eq!(declarations[1].purpose, "analytics");
    }

    #[test]
    fn a_type_with_no_purpose_still_produces_a_row() {
        // The collection is the fact. Dropping the row because the purpose is
        // absent would hide a data type the app must declare.
        let declarations =
            declarations_from(&manifest(vec![collected("name", false, false, &[])]), "url");
        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0].purpose, "unstated");
    }

    #[test]
    fn a_manifest_that_collects_nothing_produces_no_declarations() {
        // This is a real answer, not a gap. The entry exists and states that
        // the package collects nothing, which removes it from the unknown list.
        let entry = entry_from(
            "swift-package:alamofire",
            "Alamofire",
            Ecosystem::SwiftPackage,
            &manifest(vec![]),
            "url",
            Utc::now(),
        );
        assert!(entry.required_privacy_declarations.is_empty());
        assert_eq!(entry.package_id, "swift-package:alamofire");
        assert_eq!(
            entry.declarations_state,
            DeclarationsState::Curated,
            "a manifest that lists nothing is an answer"
        );
    }

    #[test]
    fn a_package_with_no_manifest_is_marked_as_an_absence() {
        // The two empty lists look the same. The state is the only thing that
        // keeps "collects nothing" apart from "nobody knows".
        let entry = entry_without_manifest(
            "swift-package:mystery",
            "Mystery",
            Ecosystem::SwiftPackage,
            Utc::now(),
        );
        assert!(entry.required_privacy_declarations.is_empty());
        assert_eq!(
            entry.declarations_state,
            DeclarationsState::NoManifestPublished
        );
    }

    #[test]
    fn every_declaration_carries_where_it_came_from() {
        // Without the url nobody can check the claim, and an unchecked claim
        // is what this whole design exists to keep off a form.
        let declarations = declarations_from(
            &manifest(vec![
                collected("device_id", true, true, &["analytics"]),
                collected("crash_data", false, false, &["app_functionality"]),
            ]),
            "https://example.test/manifest",
        );
        assert_eq!(declarations.len(), 2);
        for declaration in &declarations {
            assert!(declaration.source_url.is_some());
            assert!(declaration.provenance.expect("provenance is set").trusted());
        }
    }
}
