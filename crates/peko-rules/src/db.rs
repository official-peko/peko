//! Loading, indexing and querying the versioned rule database.
//!
//! On disk layout:
//!
//! ```text
//! rules/
//!   manifest.json          database version and metadata
//!   AAPL-PRIV.json         an array of rules, one file per platform+category
//!   GPLAY-PERM.json
//! ```

use crate::category::Category;
use crate::error::{Result, RuleError, ValidationIssue};
use crate::id::RuleId;
use crate::platform::Platform;
use crate::schema::{Rule, RuleStatus, RuleType, Severity};
use crate::validate::validate_rule;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The name of the database manifest file inside the rules directory.
pub const MANIFEST_FILE: &str = "manifest.json";

/// The rule schema version that this crate understands.
pub const SCHEMA_VERSION: u32 = 1;

/// Metadata that describes one build of the rule database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseManifest {
    /// The semantic version of the database content.
    pub database_version: String,
    /// The rule schema version.
    pub schema_version: u32,
    /// When the database was last published.
    pub updated_at: DateTime<Utc>,
    /// A short description of this build.
    #[serde(default)]
    pub description: String,
}

/// A loaded, indexed rule database.
#[derive(Debug, Clone)]
pub struct RuleDatabase {
    manifest: DatabaseManifest,
    version: semver::Version,
    rules: Vec<Rule>,
    index: HashMap<RuleId, usize>,
}

/// Filters applied to a database query. Every field is optional. A `None`
/// field does not restrict the result.
#[derive(Debug, Clone, Default)]
pub struct RuleQuery {
    pub platform: Option<Platform>,
    pub category: Option<Category>,
    pub severity: Option<Severity>,
    pub rule_type: Option<RuleType>,
    pub status: Option<RuleStatus>,
}

impl RuleQuery {
    /// A query that returns the rules the checker runs: validated rules for
    /// one platform.
    pub fn active_for(platform: Platform) -> Self {
        Self {
            platform: Some(platform),
            status: Some(RuleStatus::Validated),
            ..Self::default()
        }
    }

    fn matches(&self, rule: &Rule) -> bool {
        if let Some(platform) = self.platform {
            if !rule.applies_to_platform(platform) {
                return false;
            }
        }
        if let Some(category) = self.category {
            if rule.category != category {
                return false;
            }
        }
        if let Some(severity) = self.severity {
            if rule.severity != severity {
                return false;
            }
        }
        if let Some(rule_type) = self.rule_type {
            if rule.rule_type != rule_type {
                return false;
            }
        }
        if let Some(status) = self.status {
            if rule.status != status {
                return false;
            }
        }
        true
    }
}

impl RuleDatabase {
    /// Build a database from rules already in memory.
    pub fn new(manifest: DatabaseManifest, rules: Vec<Rule>) -> Result<Self> {
        let version = semver::Version::parse(&manifest.database_version).map_err(|source| {
            RuleError::InvalidVersion {
                version: manifest.database_version.clone(),
                source,
            }
        })?;
        let mut index = HashMap::with_capacity(rules.len());
        for (position, rule) in rules.iter().enumerate() {
            index.insert(rule.rule_id, position);
        }
        Ok(Self {
            manifest,
            version,
            rules,
            index,
        })
    }

    /// Read the database from a directory.
    pub fn load_from_dir(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let manifest_path = dir.join(MANIFEST_FILE);
        if !manifest_path.is_file() {
            return Err(RuleError::ManifestNotFound {
                path: manifest_path,
            });
        }
        let manifest: DatabaseManifest = read_json(&manifest_path)?;

        let mut rules: Vec<Rule> = Vec::new();
        let mut origin: HashMap<RuleId, PathBuf> = HashMap::new();

        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|source| RuleError::Io {
                path: dir.to_path_buf(),
                source,
            })?
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path.extension().is_some_and(|ext| ext == "json")
                    && path.file_name().is_some_and(|name| name != MANIFEST_FILE)
            })
            .collect();
        files.sort();

        for path in files {
            let batch: Vec<Rule> = read_json(&path)?;
            for rule in batch {
                if let Some(first) = origin.get(&rule.rule_id) {
                    return Err(RuleError::DuplicateRuleId {
                        id: rule.rule_id.to_string(),
                        first: first.clone(),
                        second: path.clone(),
                    });
                }
                origin.insert(rule.rule_id, path.clone());
                rules.push(rule);
            }
        }

        rules.sort_by_key(|rule| rule.rule_id);
        Self::new(manifest, rules)
    }

    pub fn manifest(&self) -> &DatabaseManifest {
        &self.manifest
    }

    /// The semantic version of the database content.
    pub fn version(&self) -> &semver::Version {
        &self.version
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.manifest.updated_at
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Every rule, in rule id order.
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Look up one rule by id.
    pub fn get(&self, id: RuleId) -> Option<&Rule> {
        self.index.get(&id).map(|position| &self.rules[*position])
    }

    /// The rules that match a query, in rule id order.
    pub fn query(&self, query: &RuleQuery) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|rule| query.matches(rule))
            .collect()
    }

    /// The validated rules that apply to `platform`.
    pub fn active_rules(&self, platform: Platform) -> Vec<&Rule> {
        self.query(&RuleQuery::active_for(platform))
    }

    /// Counts of rules by status, for reporting on the validation gate.
    pub fn status_counts(&self) -> HashMap<RuleStatus, usize> {
        let mut counts = HashMap::new();
        for rule in &self.rules {
            *counts.entry(rule.status).or_insert(0) += 1;
        }
        counts
    }

    /// Validate every rule. An empty result means the database is sound.
    pub fn validate(&self) -> Vec<ValidationIssue> {
        let mut issues: Vec<ValidationIssue> = self.rules.iter().flat_map(validate_rule).collect();
        if self.manifest.schema_version != SCHEMA_VERSION {
            issues.push(ValidationIssue {
                rule_id: "<manifest>".into(),
                field: "schema_version".into(),
                message: format!(
                    "database declares schema version {} but this build understands {SCHEMA_VERSION}",
                    self.manifest.schema_version
                ),
            });
        }
        issues.sort_by(|a, b| a.rule_id.cmp(&b.rule_id).then(a.field.cmp(&b.field)));
        issues
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path).map_err(|source| RuleError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| RuleError::Json {
        path: path.to_path_buf(),
        source,
    })
}
