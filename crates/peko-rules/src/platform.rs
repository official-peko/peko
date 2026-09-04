//! Target platform identifiers.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// The store platform that a rule or a check applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    /// Apple App Store.
    Ios,
    /// Google Play.
    Android,
    /// Both stores.
    Both,
}

impl Platform {
    /// The rule id prefix for this platform.
    pub fn prefix(self) -> &'static str {
        match self {
            Platform::Ios => "AAPL",
            Platform::Android => "GPLAY",
            Platform::Both => "BOTH",
        }
    }

    /// The lowercase name used in API requests and reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Ios => "ios",
            Platform::Android => "android",
            Platform::Both => "both",
        }
    }

    /// True if a rule for `self` must run against a submission for `target`.
    pub fn applies_to(self, target: Platform) -> bool {
        self == Platform::Both || target == Platform::Both || self == target
    }

    /// Parse a rule id prefix such as `AAPL`.
    pub fn from_prefix(prefix: &str) -> Option<Self> {
        match prefix {
            "AAPL" => Some(Platform::Ios),
            "GPLAY" => Some(Platform::Android),
            "BOTH" => Some(Platform::Both),
            _ => None,
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Platform {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "ios" | "apple" => Ok(Platform::Ios),
            "android" | "play" => Ok(Platform::Android),
            "both" => Ok(Platform::Both),
            other => Err(format!("unknown platform {other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_round_trip() {
        for platform in [Platform::Ios, Platform::Android, Platform::Both] {
            assert_eq!(Platform::from_prefix(platform.prefix()), Some(platform));
        }
    }

    #[test]
    fn both_applies_everywhere() {
        assert!(Platform::Both.applies_to(Platform::Ios));
        assert!(Platform::Both.applies_to(Platform::Android));
        assert!(Platform::Ios.applies_to(Platform::Ios));
        assert!(!Platform::Ios.applies_to(Platform::Android));
    }
}
