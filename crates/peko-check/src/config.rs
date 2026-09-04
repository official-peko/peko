//! `.pekorc.json`, the project configuration and override file.
//!
//! The file lives in the user repository under version control. It records
//! which findings a team acknowledged, who acknowledged them, and when.

use crate::error::{CheckError, Result};
use chrono::{DateTime, Utc};
use peko_rules::{Platform, RuleId, Severity};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// The file name that the CLI looks for in the project root.
pub const CONFIG_FILE: &str = ".pekorc.json";

/// The environment variable that holds the API key by default.
pub const DEFAULT_API_KEY_ENV: &str = "PEKO_API_KEY";

/// The state a user assigned to a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverrideStatus {
    /// The team read the finding and accepted the risk. The finding still
    /// appears in the report, marked as overridden, and it does not fail the
    /// build.
    Acknowledged,
    /// The finding does not apply to this project.
    NotApplicable,
}

/// One acknowledged rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleOverride {
    pub rule_id: RuleId,
    pub status: OverrideStatus,
    /// Why the team accepted the finding. Required, so the audit trail says
    /// more than "ignored".
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_at: Option<DateTime<Utc>>,
}

/// Which tier runs at which moment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierConfig {
    #[serde(default = "default_on_push")]
    pub on_push: String,
    #[serde(default = "default_on_release")]
    pub on_release: String,
}

fn default_on_push() -> String {
    "lint".into()
}

fn default_on_release() -> String {
    "audit".into()
}

impl Default for TierConfig {
    fn default() -> Self {
        Self {
            on_push: default_on_push(),
            on_release: default_on_release(),
        }
    }
}

/// The parsed `.pekorc.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PekoConfig {
    /// The config file format version.
    #[serde(default = "default_version")]
    pub version: u32,
    pub platform: Platform,
    /// The environment variable that holds the API key.
    #[serde(default = "default_api_key_env")]
    pub api_key_env: String,
    #[serde(default)]
    pub tier: TierConfig,
    #[serde(default)]
    pub overrides: Vec<RuleOverride>,
    /// Paths excluded from every scan, as globs relative to the project root.
    #[serde(default)]
    pub exclude_paths: Vec<String>,
    /// The lowest severity that fails the run.
    #[serde(default = "default_threshold")]
    pub severity_threshold: Severity,
    #[serde(default = "default_output_format")]
    pub output_format: String,
    /// Facts about the project that no file states.
    ///
    /// A checker cannot read the Kids Category from a repository, and it
    /// cannot read the countries an app ships to. A rule that depends on such
    /// a fact stays unevaluated until the developer declares it here.
    ///
    /// ```json
    /// "facts": {
    ///   "kids_category": false,
    ///   "ships_vpn": false,
    ///   "distributes_in": ["eu", "us-ca"],
    ///   "privacy_policy_url": "https://example.com/privacy"
    /// }
    /// ```
    #[serde(default)]
    pub facts: std::collections::BTreeMap<String, serde_json::Value>,
}

fn default_version() -> u32 {
    1
}

fn default_api_key_env() -> String {
    DEFAULT_API_KEY_ENV.into()
}

fn default_threshold() -> Severity {
    Severity::Warning
}

fn default_output_format() -> String {
    "json".into()
}

impl PekoConfig {
    /// A default configuration for a platform.
    pub fn new(platform: Platform) -> Self {
        Self {
            version: default_version(),
            platform,
            api_key_env: default_api_key_env(),
            tier: TierConfig::default(),
            overrides: Vec::new(),
            exclude_paths: vec!["Tests/**".into(), "**/Pods/**".into(), "**/build/**".into()],
            severity_threshold: default_threshold(),
            output_format: default_output_format(),
            facts: std::collections::BTreeMap::new(),
        }
    }

    /// Read one declared fact.
    pub fn fact(&self, key: &str) -> Option<&serde_json::Value> {
        self.facts.get(key)
    }

    /// Read the configuration from a file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|source| CheckError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(path, &text)
    }

    /// Parse configuration text.
    pub fn parse(path: &Path, text: &str) -> Result<Self> {
        serde_json::from_str(text).map_err(|source| CheckError::Config {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Read the configuration from a project root. A missing file yields the
    /// default configuration for `platform`.
    pub fn load_or_default(root: &Path, platform: Platform) -> Result<Self> {
        let path = root.join(CONFIG_FILE);
        if path.is_file() {
            Self::load(&path)
        } else {
            Ok(Self::new(platform))
        }
    }

    /// Index the overrides by rule id.
    pub fn override_map(&self) -> HashMap<RuleId, &RuleOverride> {
        self.overrides
            .iter()
            .map(|entry| (entry.rule_id, entry))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_specification_example() {
        let text = r#"{
  "version": 1,
  "platform": "ios",
  "api_key_env": "PEKO_API_KEY",
  "tier": { "on_push": "lint", "on_release": "audit" },
  "overrides": [
    {
      "rule_id": "AAPL-CONTENT-007",
      "status": "acknowledged",
      "reason": "App content was reviewed by legal counsel",
      "acknowledged_by": "preston@pekoui.com",
      "acknowledged_at": "2026-08-15T10:00:00Z"
    }
  ],
  "exclude_paths": ["Tests/", "Fixtures/"],
  "severity_threshold": "warning",
  "output_format": "json"
}"#;
        let config = PekoConfig::parse(Path::new(".pekorc.json"), text).unwrap();
        assert_eq!(config.platform, Platform::Ios);
        assert_eq!(config.overrides.len(), 1);
        assert_eq!(config.severity_threshold, Severity::Warning);
        let map = config.override_map();
        let id: RuleId = "AAPL-CONTENT-007".parse().unwrap();
        assert_eq!(map[&id].status, OverrideStatus::Acknowledged);
    }

    #[test]
    fn facts_are_read_from_the_file() {
        let text = r#"{
          "platform": "ios",
          "facts": {
            "kids_category": false,
            "distributes_in": ["eu", "us-ca"],
            "privacy_policy_url": "https://example.com/privacy"
          }
        }"#;
        let config = PekoConfig::parse(Path::new(".pekorc.json"), text).unwrap();
        assert_eq!(
            config.fact("kids_category"),
            Some(&serde_json::json!(false))
        );
        assert_eq!(
            config.fact("distributes_in"),
            Some(&serde_json::json!(["eu", "us-ca"]))
        );
        assert!(
            config.fact("ships_vpn").is_none(),
            "an undeclared fact is unknown"
        );
    }

    #[test]
    fn fills_in_defaults() {
        let config =
            PekoConfig::parse(Path::new(".pekorc.json"), r#"{"platform":"android"}"#).unwrap();
        assert_eq!(config.version, 1);
        assert_eq!(config.api_key_env, DEFAULT_API_KEY_ENV);
        assert_eq!(config.severity_threshold, Severity::Warning);
        assert_eq!(config.tier.on_release, "audit");
    }

    #[test]
    fn rejects_an_unknown_platform() {
        assert!(PekoConfig::parse(Path::new(".pekorc.json"), r#"{"platform":"web"}"#).is_err());
    }
}
