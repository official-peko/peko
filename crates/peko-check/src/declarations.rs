//! Turning a dependency list into the privacy answers both stores ask for.
//!
//! A developer fills two forms by hand today: the App Privacy nutrition label
//! on App Store Connect, and the Data safety form on Play Console. Both ask
//! about behaviour that lives inside the SDKs the app links, not inside the
//! code the developer wrote. That is why the answers are so often wrong, and a
//! wrong answer is its own rejection.
//!
//! This module reads the resolved dependencies, looks each one up in the
//! knowledge base, and reports what the developer must declare.
//!
//! It reports three things apart from each other, and the separation is the
//! point:
//!
//! - answers, where a curated entry states the behaviour
//! - gaps, where the entry exists but nobody curated the field
//! - unknown packages, where no entry exists at all
//!
//! A mapper that folded a gap into "no" would hand somebody a form that reads
//! complete and is false. The store finds that out, not the developer.

use crate::knowledge::{Collection, KnowledgeBase, Optionality};
use peko_parse::deps::{Dependency, Scope};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One row of the iOS App Privacy nutrition label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IosDataType {
    pub data_type: String,
    /// Every purpose any linked package needs this data for.
    pub purposes: BTreeSet<String>,
    /// True when any package links the data to the user's identity.
    ///
    /// Any is the right reduction. One package that links it makes the whole
    /// answer "linked", and a developer who answers "not linked" because the
    /// other four do not is wrong.
    pub linked_to_identity: bool,
    /// None when no package that declares this data type states the field.
    pub used_for_tracking: Option<bool>,
    /// The packages that put this row on the form.
    pub because_of: BTreeSet<String>,
    /// True when any package behind this row is a draft nobody checked.
    ///
    /// The row still shows. A reader has to see that part of it rests on a
    /// guess, because the row reads identical to a verified one otherwise.
    pub unverified: bool,
}

/// One row of the Play Data safety form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidDataType {
    pub data_type: String,
    pub purposes: BTreeSet<String>,
    /// None when no package that declares this data type states the field.
    pub collection: Option<Collection>,
    pub optionality: Option<Optionality>,
    pub because_of: BTreeSet<String>,
    /// True when any package behind this row is a draft nobody checked.
    pub unverified: bool,
}

/// A field that a curated entry leaves unanswered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gap {
    pub package_id: String,
    pub data_type: String,
    /// The field name, as the form asks it.
    pub field: String,
}

/// What the developer must declare, and what is still unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclarationReport {
    pub ios: Vec<IosDataType>,
    pub android: Vec<AndroidDataType>,
    /// Curated entries with a field nobody filled in.
    pub gaps: Vec<Gap>,
    /// Dependencies with no knowledge base entry at all.
    pub unknown_packages: Vec<String>,
    /// How many dependencies had an entry that says something.
    pub known_packages: usize,
    /// Packages whose answers are a draft nobody checked.
    ///
    /// These count inside `known_packages`, because they do say something.
    /// They are listed apart so a reader can see how much of the form rests
    /// on a guess.
    pub unverified_packages: Vec<String>,
}

impl DeclarationReport {
    /// True when every field of every row has an answer.
    ///
    /// A report that is not complete must not be pasted into a form as is.
    #[must_use]
    pub fn complete(&self) -> bool {
        self.gaps.is_empty() && self.unknown_packages.is_empty() && self.verified()
    }

    /// True when every answer rests on evidence rather than a draft.
    #[must_use]
    pub fn verified(&self) -> bool {
        self.unverified_packages.is_empty()
    }

    /// The share of dependencies the knowledge base knows.
    ///
    /// Returned as a fraction, never as a count. A bare "62 packages known"
    /// hides the denominator that decides whether the answer is usable.
    #[must_use]
    pub fn coverage(&self) -> (usize, usize) {
        (
            self.known_packages,
            self.known_packages + self.unknown_packages.len(),
        )
    }
}

/// Fold one package's declarations into the two forms.
fn merge(
    package_id: &str,
    verified: bool,
    declarations: &[crate::knowledge::PrivacyDeclaration],
    ios: &mut BTreeMap<String, IosDataType>,
    android: &mut BTreeMap<String, AndroidDataType>,
    gaps: &mut Vec<Gap>,
) {
    for declaration in declarations {
        let row = ios
            .entry(declaration.data_type.clone())
            .or_insert_with(|| IosDataType {
                data_type: declaration.data_type.clone(),
                purposes: BTreeSet::new(),
                linked_to_identity: false,
                used_for_tracking: None,
                because_of: BTreeSet::new(),
                unverified: false,
            });
        row.purposes.insert(declaration.purpose.clone());
        row.linked_to_identity |= declaration.linked_to_identity;
        row.unverified |= !verified;
        row.because_of.insert(package_id.to_string());
        match declaration.used_for_tracking {
            // Any package that tracks makes the whole row tracked.
            Some(true) => row.used_for_tracking = Some(true),
            Some(false) => {
                if row.used_for_tracking.is_none() {
                    row.used_for_tracking = Some(false);
                }
            }
            None => gaps.push(Gap {
                package_id: package_id.to_string(),
                data_type: declaration.data_type.clone(),
                field: "used_for_tracking".to_string(),
            }),
        }

        let row = android
            .entry(declaration.data_type.clone())
            .or_insert_with(|| AndroidDataType {
                data_type: declaration.data_type.clone(),
                purposes: BTreeSet::new(),
                collection: None,
                optionality: None,
                because_of: BTreeSet::new(),
                unverified: false,
            });
        row.purposes.insert(declaration.purpose.clone());
        row.because_of.insert(package_id.to_string());
        row.unverified |= !verified;
        match declaration.collection {
            Some(value) => {
                row.collection = Some(match row.collection {
                    // The first package to name this data type sets the answer.
                    None => value,
                    // Two packages that agree keep the answer they agree on.
                    Some(held) if held == value => held,
                    // They disagree, so the app does both. Collected by one
                    // and shared by another is collected and shared.
                    Some(_) => Collection::Both,
                });
            }
            None => gaps.push(Gap {
                package_id: package_id.to_string(),
                data_type: declaration.data_type.clone(),
                field: "collection".to_string(),
            }),
        }
        match declaration.optionality {
            // Required by one package is required for the app.
            Some(Optionality::Required) => row.optionality = Some(Optionality::Required),
            Some(Optionality::Optional) => {
                if row.optionality.is_none() {
                    row.optionality = Some(Optionality::Optional);
                }
            }
            None => gaps.push(Gap {
                package_id: package_id.to_string(),
                data_type: declaration.data_type.clone(),
                field: "optionality".to_string(),
            }),
        }
    }
}

/// Build the declaration report for one dependency list.
#[must_use]
pub fn map_declarations(
    dependencies: &[Dependency],
    knowledge: &KnowledgeBase,
) -> DeclarationReport {
    let mut ios: BTreeMap<String, IosDataType> = BTreeMap::new();
    let mut android: BTreeMap<String, AndroidDataType> = BTreeMap::new();
    let mut gaps = Vec::new();
    let mut unknown = BTreeSet::new();
    let mut unverified = BTreeSet::new();
    let mut known = BTreeSet::new();

    for dependency in dependencies {
        // A test dependency never reaches a user, so it collects nothing in
        // production. Counting one would put a row on the form that the app
        // does not earn, and would report a package as unknown that no
        // curator ever needs to read.
        if dependency.scope == Scope::TestOnly {
            continue;
        }
        match knowledge.get(&dependency.package_id) {
            // An entry that only records "the vendor publishes no manifest"
            // knows nothing about what the package collects. Counting it as
            // known would report an empty form as a complete one.
            // An entry that establishes nothing is not knowledge. A vendor
            // that publishes no manifest, and an entry nobody reviewed, both
            // land here. Counting either as known would report an empty form
            // as a complete one.
            Some(entry) if !entry.states_what_it_collects() => {
                unknown.insert(dependency.package_id.clone());
            }
            Some(entry) => {
                known.insert(dependency.package_id.clone());
                if !entry.declarations_state.verified() {
                    unverified.insert(dependency.package_id.clone());
                }
                merge(
                    &dependency.package_id,
                    entry.declarations_state.verified(),
                    &entry.required_privacy_declarations,
                    &mut ios,
                    &mut android,
                    &mut gaps,
                );
            }
            None => {
                unknown.insert(dependency.package_id.clone());
            }
        }
    }

    DeclarationReport {
        ios: ios.into_values().collect(),
        android: android.into_values().collect(),
        gaps,
        unknown_packages: unknown.into_iter().collect(),
        known_packages: known.len(),
        unverified_packages: unverified.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{DeclarationsState, PackageEntry, PrivacyDeclaration};
    use chrono::Utc;
    use peko_rules::Ecosystem;
    use std::path::PathBuf;

    fn declaration(
        data_type: &str,
        purpose: &str,
        linked: bool,
        tracking: Option<bool>,
        collection: Option<Collection>,
        optionality: Option<Optionality>,
    ) -> PrivacyDeclaration {
        PrivacyDeclaration {
            data_type: data_type.to_string(),
            purpose: purpose.to_string(),
            linked_to_identity: linked,
            used_for_tracking: tracking,
            collection,
            optionality,
            provenance: None,
            source_url: None,
        }
    }

    fn entry(package_id: &str, declarations: Vec<PrivacyDeclaration>) -> PackageEntry {
        PackageEntry {
            package_id: package_id.to_string(),
            package_name: package_id.to_string(),
            ecosystem: Ecosystem::Cocoapods,
            compliance_flags: Vec::new(),
            required_privacy_declarations: declarations,
            declarations_state: DeclarationsState::Curated,
            last_updated: Utc::now(),
        }
    }

    fn dependency(package_id: &str) -> Dependency {
        Dependency {
            package_id: package_id.to_string(),
            name: package_id.to_string(),
            ecosystem: Ecosystem::Cocoapods,
            version: None,
            declared_in: PathBuf::from("Podfile.lock"),
            line: None,
            scope: Scope::Ships,
        }
    }

    fn test_dependency(package_id: &str) -> Dependency {
        Dependency {
            scope: Scope::TestOnly,
            ..dependency(package_id)
        }
    }

    fn drafted_entry(package_id: &str, declarations: Vec<PrivacyDeclaration>) -> PackageEntry {
        PackageEntry {
            declarations_state: DeclarationsState::ModelDrafted,
            ..entry(package_id, declarations)
        }
    }

    #[test]
    fn a_model_draft_answers_but_never_reads_as_verified() {
        // The draft is useful. It is not evidence. A report that folded the
        // two together would put an unchecked guess on a submitted form, and
        // the developer would never see which half to check.
        let base = KnowledgeBase::new(vec![drafted_entry(
            "cocoapods:Guessed",
            vec![declaration(
                "device_id",
                "analytics",
                true,
                Some(false),
                Some(Collection::Collected),
                Some(Optionality::Optional),
            )],
        )]);
        let report = map_declarations(&[dependency("cocoapods:Guessed")], &base);
        assert_eq!(report.coverage(), (1, 1), "a draft still answers");
        assert!(report.gaps.is_empty(), "a draft fills every field");
        assert!(!report.verified(), "a draft read as evidence");
        assert!(!report.complete(), "a draft read as a finished form");
        assert_eq!(report.unverified_packages, vec!["cocoapods:Guessed"]);
        assert!(
            report.ios[0].unverified,
            "the row hid that it rests on a draft"
        );
        assert!(report.android[0].unverified);
    }

    #[test]
    fn one_drafted_package_marks_only_the_rows_it_touches() {
        // A form built from twenty curated packages and one draft must show
        // exactly which rows the draft reached, not blanket the whole report.
        let base = KnowledgeBase::new(vec![
            entry(
                "cocoapods:Solid",
                vec![declaration(
                    "crash_data",
                    "app_functionality",
                    false,
                    Some(false),
                    Some(Collection::Collected),
                    Some(Optionality::Optional),
                )],
            ),
            drafted_entry(
                "cocoapods:Guessed",
                vec![declaration(
                    "location",
                    "analytics",
                    false,
                    Some(false),
                    Some(Collection::Collected),
                    Some(Optionality::Optional),
                )],
            ),
        ]);
        let report = map_declarations(
            &[
                dependency("cocoapods:Solid"),
                dependency("cocoapods:Guessed"),
            ],
            &base,
        );
        let crash = report
            .ios
            .iter()
            .find(|r| r.data_type == "crash_data")
            .expect("row");
        let location = report
            .ios
            .iter()
            .find(|r| r.data_type == "location")
            .expect("row");
        assert!(!crash.unverified, "a curated row was marked as a draft");
        assert!(location.unverified, "a drafted row was not marked");
    }

    #[test]
    fn a_shared_row_is_unverified_when_any_contributor_is_a_draft() {
        // Two packages name the same data type. One is evidence and one is a
        // guess. The row rests partly on the guess, so it must say so.
        let base = KnowledgeBase::new(vec![
            entry(
                "cocoapods:Solid",
                vec![declaration(
                    "device_id",
                    "app_functionality",
                    false,
                    Some(false),
                    Some(Collection::Collected),
                    Some(Optionality::Optional),
                )],
            ),
            drafted_entry(
                "cocoapods:Guessed",
                vec![declaration(
                    "device_id",
                    "analytics",
                    true,
                    Some(true),
                    Some(Collection::Shared),
                    Some(Optionality::Required),
                )],
            ),
        ]);
        let report = map_declarations(
            &[
                dependency("cocoapods:Solid"),
                dependency("cocoapods:Guessed"),
            ],
            &base,
        );
        assert_eq!(report.ios.len(), 1);
        assert!(
            report.ios[0].unverified,
            "a row built partly on a draft read as verified"
        );
    }

    #[test]
    fn a_package_whose_vendor_publishes_no_manifest_stays_unknown() {
        // The entry exists and its declaration list is empty. That empty list
        // is an absence, not an answer, so the package must still read as
        // unknown. Otherwise the report says the form is complete.
        let base = KnowledgeBase::new(vec![crate::knowledge::PackageEntry {
            package_id: "cocoapods:Silent".to_string(),
            package_name: "Silent".to_string(),
            ecosystem: Ecosystem::Cocoapods,
            compliance_flags: Vec::new(),
            required_privacy_declarations: Vec::new(),
            declarations_state: DeclarationsState::NoManifestPublished,
            last_updated: Utc::now(),
        }]);
        let report = map_declarations(&[dependency("cocoapods:Silent")], &base);
        assert_eq!(report.unknown_packages, vec!["cocoapods:Silent"]);
        assert!(
            !report.complete(),
            "an absence of evidence read as a clean form"
        );
        assert_eq!(report.coverage(), (0, 1));
    }

    #[test]
    fn a_test_dependency_reaches_no_user_so_it_reaches_no_form() {
        // junit and espresso sit in almost every Android build file. They run
        // on a build machine. Counting them as unknown packages made the
        // coverage number read far worse than the truth.
        let report = map_declarations(
            &[test_dependency("gradle:junit:junit")],
            &KnowledgeBase::default(),
        );
        assert!(
            report.unknown_packages.is_empty(),
            "a test dependency reached the report"
        );
        assert_eq!(report.coverage(), (0, 0));
        assert!(report.complete(), "a test only list read as incomplete");
    }

    #[test]
    fn a_package_nobody_curated_is_reported_as_unknown_not_as_clean() {
        // Silence here would tell a developer the app declares nothing. The
        // dependency is still in the binary, and the store still sees it.
        let report = map_declarations(
            &[dependency("cocoapods:Mystery")],
            &KnowledgeBase::default(),
        );
        assert!(report.ios.is_empty());
        assert_eq!(report.unknown_packages, vec!["cocoapods:Mystery"]);
        assert!(
            !report.complete(),
            "an unknown package read as a complete form"
        );
        assert_eq!(report.coverage(), (0, 1));
    }

    #[test]
    fn an_uncurated_field_becomes_a_gap_not_a_no() {
        // This is the failure the Option guards. "Not used for tracking" is an
        // answer. "Nobody checked" is not, and printing the first for the
        // second is how a form goes out wrong.
        let base = KnowledgeBase::new(vec![entry(
            "cocoapods:Thing",
            vec![declaration(
                "device_id",
                "analytics",
                true,
                None,
                None,
                None,
            )],
        )]);
        let report = map_declarations(&[dependency("cocoapods:Thing")], &base);
        assert_eq!(report.ios[0].used_for_tracking, None);
        assert_eq!(report.android[0].collection, None);
        assert_eq!(report.gaps.len(), 3, "three unanswered fields");
        assert!(!report.complete());
    }

    #[test]
    fn one_tracking_package_makes_the_row_tracked() {
        // Four packages that do not track and one that does still means the
        // app tracks. Answering "no" because most do not is the common error.
        let base = KnowledgeBase::new(vec![
            entry(
                "cocoapods:Quiet",
                vec![declaration(
                    "device_id",
                    "app_functionality",
                    false,
                    Some(false),
                    Some(Collection::Collected),
                    Some(Optionality::Optional),
                )],
            ),
            entry(
                "cocoapods:Tracker",
                vec![declaration(
                    "device_id",
                    "advertising",
                    true,
                    Some(true),
                    Some(Collection::Shared),
                    Some(Optionality::Required),
                )],
            ),
        ]);
        let report = map_declarations(
            &[
                dependency("cocoapods:Quiet"),
                dependency("cocoapods:Tracker"),
            ],
            &base,
        );
        assert_eq!(report.ios.len(), 1, "one data type, one row");
        assert_eq!(report.ios[0].used_for_tracking, Some(true));
        assert!(
            report.ios[0].linked_to_identity,
            "one linked package linked the row"
        );
        assert_eq!(report.ios[0].purposes.len(), 2);
        assert_eq!(report.ios[0].because_of.len(), 2);
    }

    #[test]
    fn a_linked_package_listed_first_still_links_the_row() {
        // Order must not decide the answer. The linked package comes first
        // here and the unlinked one second, so a reduction that keeps the last
        // value reports "not linked" and the form goes out wrong.
        let base = KnowledgeBase::new(vec![
            entry(
                "cocoapods:Linked",
                vec![declaration(
                    "device_id",
                    "account",
                    true,
                    Some(false),
                    Some(Collection::Collected),
                    Some(Optionality::Optional),
                )],
            ),
            entry(
                "cocoapods:Unlinked",
                vec![declaration(
                    "device_id",
                    "app_functionality",
                    false,
                    Some(false),
                    Some(Collection::Collected),
                    Some(Optionality::Optional),
                )],
            ),
        ]);
        let report = map_declarations(
            &[
                dependency("cocoapods:Linked"),
                dependency("cocoapods:Unlinked"),
            ],
            &base,
        );
        assert!(
            report.ios[0].linked_to_identity,
            "the second package unlinked a row the first linked"
        );
    }

    #[test]
    fn a_tracking_package_listed_first_still_tracks_the_row() {
        // The same order trap, on the tracking column.
        let base = KnowledgeBase::new(vec![
            entry(
                "cocoapods:Tracks",
                vec![declaration(
                    "device_id",
                    "advertising",
                    false,
                    Some(true),
                    Some(Collection::Shared),
                    Some(Optionality::Optional),
                )],
            ),
            entry(
                "cocoapods:Quiet",
                vec![declaration(
                    "device_id",
                    "app_functionality",
                    false,
                    Some(false),
                    Some(Collection::Collected),
                    Some(Optionality::Optional),
                )],
            ),
        ]);
        let report = map_declarations(
            &[
                dependency("cocoapods:Tracks"),
                dependency("cocoapods:Quiet"),
            ],
            &base,
        );
        assert_eq!(report.ios[0].used_for_tracking, Some(true));
    }

    #[test]
    fn collected_by_one_and_shared_by_another_is_both() {
        let base = KnowledgeBase::new(vec![
            entry(
                "cocoapods:A",
                vec![declaration(
                    "location",
                    "app_functionality",
                    false,
                    Some(false),
                    Some(Collection::Collected),
                    Some(Optionality::Optional),
                )],
            ),
            entry(
                "cocoapods:B",
                vec![declaration(
                    "location",
                    "advertising",
                    false,
                    Some(false),
                    Some(Collection::Shared),
                    Some(Optionality::Optional),
                )],
            ),
        ]);
        let report = map_declarations(
            &[dependency("cocoapods:A"), dependency("cocoapods:B")],
            &base,
        );
        assert_eq!(report.android[0].collection, Some(Collection::Both));
    }

    #[test]
    fn both_survives_a_third_package_that_only_collects() {
        // Once two packages disagree the row is Both. A third package that
        // only collects must not narrow it back to Collected.
        let make = |id: &str, collection: Collection| {
            entry(
                id,
                vec![declaration(
                    "location",
                    "app_functionality",
                    false,
                    Some(false),
                    Some(collection),
                    Some(Optionality::Optional),
                )],
            )
        };
        let base = KnowledgeBase::new(vec![
            make("cocoapods:A", Collection::Collected),
            make("cocoapods:B", Collection::Shared),
            make("cocoapods:C", Collection::Collected),
        ]);
        let report = map_declarations(
            &[
                dependency("cocoapods:A"),
                dependency("cocoapods:B"),
                dependency("cocoapods:C"),
            ],
            &base,
        );
        assert_eq!(report.android[0].collection, Some(Collection::Both));
    }

    #[test]
    fn required_by_one_package_makes_the_row_required() {
        let base = KnowledgeBase::new(vec![
            entry(
                "cocoapods:A",
                vec![declaration(
                    "email",
                    "account",
                    true,
                    Some(false),
                    Some(Collection::Collected),
                    Some(Optionality::Optional),
                )],
            ),
            entry(
                "cocoapods:B",
                vec![declaration(
                    "email",
                    "account",
                    true,
                    Some(false),
                    Some(Collection::Collected),
                    Some(Optionality::Required),
                )],
            ),
        ]);
        let report = map_declarations(
            &[dependency("cocoapods:A"), dependency("cocoapods:B")],
            &base,
        );
        assert_eq!(report.android[0].optionality, Some(Optionality::Required));
    }

    #[test]
    fn a_fully_curated_list_reports_complete() {
        let base = KnowledgeBase::new(vec![entry(
            "cocoapods:Clean",
            vec![declaration(
                "crash_data",
                "app_functionality",
                false,
                Some(false),
                Some(Collection::Collected),
                Some(Optionality::Optional),
            )],
        )]);
        let report = map_declarations(&[dependency("cocoapods:Clean")], &base);
        assert!(report.complete(), "a curated list still reported a gap");
        assert_eq!(report.coverage(), (1, 1));
    }

    #[test]
    fn the_same_package_listed_twice_counts_once() {
        // A Podfile.lock can name a package in two targets. Counting it twice
        // would overstate coverage.
        let base = KnowledgeBase::new(vec![entry(
            "cocoapods:Twice",
            vec![declaration(
                "device_id",
                "analytics",
                false,
                Some(false),
                Some(Collection::Collected),
                Some(Optionality::Optional),
            )],
        )]);
        let report = map_declarations(
            &[dependency("cocoapods:Twice"), dependency("cocoapods:Twice")],
            &base,
        );
        assert_eq!(report.coverage(), (1, 1));
        assert_eq!(report.ios[0].because_of.len(), 1);
    }
}
