//! Rule identifiers, for example `AAPL-PRIV-001`.

use crate::category::Category;
use crate::error::RuleError;
use crate::platform::Platform;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// A parsed rule id of the form `{PLATFORM}-{CATEGORY}-{NUMBER}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuleId {
    platform: Platform,
    category: Category,
    number: u32,
}

impl RuleId {
    /// The number of digits in the numeric part of a rule id.
    pub const NUMBER_WIDTH: usize = 3;

    /// Build a rule id from its parts.
    pub fn new(platform: Platform, category: Category, number: u32) -> Self {
        Self {
            platform,
            category,
            number,
        }
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    pub fn category(&self) -> Category {
        self.category
    }

    pub fn number(&self) -> u32 {
        self.number
    }
}

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}-{}-{:0width$}",
            self.platform.prefix(),
            self.category.code(),
            self.number,
            width = Self::NUMBER_WIDTH
        )
    }
}

impl FromStr for RuleId {
    type Err = RuleError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = |reason: &str| RuleError::InvalidRuleId {
            id: s.to_string(),
            reason: reason.to_string(),
        };

        let mut parts = s.split('-');
        let platform_part = parts.next().ok_or_else(|| invalid("missing platform"))?;
        let category_part = parts.next().ok_or_else(|| invalid("missing category"))?;
        let number_part = parts.next().ok_or_else(|| invalid("missing number"))?;
        if parts.next().is_some() {
            return Err(invalid("expected exactly three dash separated parts"));
        }

        let platform = Platform::from_prefix(platform_part)
            .ok_or_else(|| invalid("platform prefix must be one of AAPL, GPLAY, BOTH"))?;
        let category = Category::from_str(category_part).map_err(|e| invalid(&e))?;

        if number_part.len() != Self::NUMBER_WIDTH
            || !number_part.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(invalid("number must be exactly three digits"));
        }
        let number: u32 = number_part
            .parse()
            .map_err(|_| invalid("number is not an integer"))?;

        Ok(RuleId::new(platform, category, number))
    }
}

impl Serialize for RuleId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for RuleId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        RuleId::from_str(&raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_renders() {
        let id: RuleId = "AAPL-PRIV-001".parse().unwrap();
        assert_eq!(id.platform(), Platform::Ios);
        assert_eq!(id.category(), Category::Priv);
        assert_eq!(id.number(), 1);
        assert_eq!(id.to_string(), "AAPL-PRIV-001");
    }

    #[test]
    fn rejects_bad_shapes() {
        for bad in [
            "AAPL-PRIV",
            "AAPL-PRIV-1",
            "AAPL-PRIV-0001",
            "APPLE-PRIV-001",
            "AAPL-NOPE-001",
            "AAPL-PRIV-abc",
            "AAPL-PRIV-001-2",
        ] {
            assert!(bad.parse::<RuleId>().is_err(), "{bad} must not parse");
        }
    }

    #[test]
    fn serde_round_trip() {
        let id: RuleId = "GPLAY-PERM-014".parse().unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"GPLAY-PERM-014\"");
        assert_eq!(serde_json::from_str::<RuleId>(&json).unwrap(), id);
    }
}
