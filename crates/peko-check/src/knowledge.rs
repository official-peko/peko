//! The dependency knowledge base (specification section 5.3).
//!
//! Entries are curated by hand. The checker reads the base, it never writes
//! it. An unknown dependency is recorded, not flagged, and the weekly report
//! of unknown dependencies guides which entries to add next.

use crate::error::{CheckError, Result};
use chrono::{DateTime, Utc};
use peko_rules::{DependencyFlagType, Ecosystem, Platform, RuleId, Severity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One compliance relevant behavior of a package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComplianceFlag {
    pub flag_type: DependencyFlagType,
    pub description: String,
    pub severity: Severity,
    pub platform: Platform,
    #[serde(default)]
    pub related_rule_ids: Vec<RuleId>,
    /// Where this flag comes from.
    pub evidence: String,
    pub last_verified: DateTime<Utc>,
}

/// Whether a package collects the data, shares it onward, or both.
///
/// The Play data safety form asks these as two separate questions, and a
/// wrong answer there is its own rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Collection {
    /// The data leaves the device and the developer keeps it.
    Collected,
    /// The data goes to a third party.
    Shared,
    Both,
}

/// Whether a user can use the app without giving the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Optionality {
    Required,
    Optional,
}

/// How we know a declaration is true.
///
/// The field exists so a reader can tell an authoritative answer from a
/// guess. A privacy form filled from a draft nobody checked is the failure
/// this product exists to prevent, so the origin travels with the claim
/// rather than sitting in a changelog nobody reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// The SDK's own `PrivacyInfo.xcprivacy`. Apple requires it, and the
    /// vendor writes it, so it is first party evidence.
    ApplePrivacyManifest,
    /// A person read the vendor's documentation and wrote the entry.
    HandCurated,
    /// A model drafted it. Nobody checked it. Not fit for a submitted form.
    ModelDraft,
}

impl Provenance {
    /// Whether an answer from this source may go on a form as it stands.
    #[must_use]
    pub fn trusted(self) -> bool {
        matches!(self, Self::ApplePrivacyManifest | Self::HandCurated)
    }
}

/// A privacy declaration that a package makes necessary.
///
/// The three fields below are `Option` on purpose. A form answer of "no" and
/// an answer nobody curated are different things, and a mapper that prints
/// "not used for tracking" for a package it knows nothing about tells the
/// developer a lie that the store then rejects. `None` means unknown, and the
/// mapper reports it as a gap rather than an answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivacyDeclaration {
    /// For example `device_id`, `location`, `contacts`.
    pub data_type: String,
    pub purpose: String,
    pub linked_to_identity: bool,
    /// The tracking column of the iOS nutrition label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_for_tracking: Option<bool>,
    /// The collected or shared question on the Play data safety form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<Collection>,
    /// The required or optional question on the Play data safety form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optionality: Option<Optionality>,
    /// How we know. `None` means the entry predates provenance tracking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
    /// Where the evidence sits, so a reader can check it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

/// How complete the declaration list is.
///
/// An empty list is ambiguous on its own. It means "this package collects
/// nothing" when a curator checked, and it means "nobody knows" when the
/// vendor publishes no manifest. The mapper must not read the second as the
/// first, so the entry states which it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclarationsState {
    /// Nobody established what this package collects.
    ///
    /// This is the default on purpose. A state that claims something must be
    /// set by whoever established it, so an entry that nobody reviewed cannot
    /// pass itself off as a checked one.
    #[default]
    Unreviewed,
    /// The list is the complete answer. An empty list means it collects
    /// nothing, and a curator or a manifest said so.
    Curated,
    /// A model drafted the declarations. Nobody checked them.
    ///
    /// The list holds real rows, so the entry says something. It is not fit
    /// for a submitted form, and the report keeps it apart from a curated
    /// answer rather than fold the two together.
    ModelDrafted,
    /// The vendor publishes no privacy manifest, so what it collects is
    /// unknown. An empty list here is an absence of evidence, not an answer.
    ///
    /// This is also a finding in its own right. Apple requires a manifest from
    /// the SDKs on its list, so a package without one is a risk to the app
    /// that ships it.
    NoManifestPublished,
}

impl DeclarationsState {
    /// Whether the entry's declaration list is an answer rather than a blank.
    #[must_use]
    pub fn is_answer(self) -> bool {
        matches!(self, Self::Curated | Self::ModelDrafted)
    }

    /// Whether a person may put this answer on a submitted form.
    ///
    /// A draft says something useful and is still not evidence. The two are
    /// different questions, and folding them together is how an unchecked
    /// guess reaches a store.
    #[must_use]
    pub fn verified(self) -> bool {
        matches!(self, Self::Curated)
    }
}

/// One package in the knowledge base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageEntry {
    /// The ecosystem qualified id, for example `cocoapods:AFNetworking`.
    pub package_id: String,
    pub package_name: String,
    pub ecosystem: Ecosystem,
    #[serde(default)]
    pub compliance_flags: Vec<ComplianceFlag>,
    #[serde(default)]
    pub required_privacy_declarations: Vec<PrivacyDeclaration>,
    /// Whether the declaration list above is an answer or an absence.
    #[serde(default)]
    pub declarations_state: DeclarationsState,
    pub last_updated: DateTime<Utc>,
}

impl PackageEntry {
    /// Whether this entry says what the package collects.
    ///
    /// A hand curated entry from before the state field carries declarations
    /// and no state, and it still answers. An entry with neither declarations
    /// nor a `Curated` state answers nothing, and the mapper must treat it as
    /// unknown rather than as clean.
    #[must_use]
    pub fn states_what_it_collects(&self) -> bool {
        !self.required_privacy_declarations.is_empty() || self.declarations_state.is_answer()
    }
}

/// The loaded knowledge base, indexed by package id.
#[derive(Debug, Clone, Default)]
pub struct KnowledgeBase {
    entries: BTreeMap<String, PackageEntry>,
}

impl KnowledgeBase {
    pub fn new(entries: Vec<PackageEntry>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|entry| (entry.package_id.clone(), entry))
                .collect(),
        }
    }

    /// Read every `.json` file in a directory. Each file holds an array of
    /// entries.
    pub fn load_from_dir(dir: &Path) -> Result<Self> {
        let mut entries = Vec::new();
        let read = std::fs::read_dir(dir).map_err(|source| CheckError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let mut files: Vec<PathBuf> = read
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        files.sort();

        for path in files {
            let text = std::fs::read_to_string(&path).map_err(|source| CheckError::Io {
                path: path.clone(),
                source,
            })?;
            let batch: Vec<PackageEntry> =
                serde_json::from_str(&text).map_err(|source| CheckError::Config {
                    path: path.clone(),
                    source,
                })?;
            entries.extend(batch);
        }
        Ok(Self::new(entries))
    }

    pub fn get(&self, package_id: &str) -> Option<&PackageEntry> {
        self.entries.get(package_id)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = &PackageEntry> {
        self.entries.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_entry() {
        let text = r#"[{
          "package_id": "cocoapods:FirebaseAnalytics",
          "package_name": "FirebaseAnalytics",
          "ecosystem": "cocoapods",
          "compliance_flags": [{
            "flag_type": "data_collection",
            "description": "Collects device identifiers for analytics.",
            "severity": "warning",
            "platform": "ios",
            "related_rule_ids": ["AAPL-PRIV-001"],
            "evidence": "Firebase documentation, data collection page",
            "last_verified": "2026-08-01T00:00:00Z"
          }],
          "required_privacy_declarations": [{
            "data_type": "device_id",
            "purpose": "analytics",
            "linked_to_identity": false
          }],
          "last_updated": "2026-08-01T00:00:00Z"
        }]"#;
        let entries: Vec<PackageEntry> = serde_json::from_str(text).unwrap();
        let base = KnowledgeBase::new(entries);
        let entry = base.get("cocoapods:FirebaseAnalytics").unwrap();
        assert_eq!(
            entry.compliance_flags[0].flag_type,
            DependencyFlagType::DataCollection
        );
        assert_eq!(base.len(), 1);
    }
}
