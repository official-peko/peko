//! The v1 category taxonomy (specification section 4.4).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A compliance category. The code is the token used in a rule id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Category {
    /// Privacy manifests, data collection disclosures, tracking transparency.
    Priv,
    /// Objectionable content, user generated content moderation.
    Content,
    /// In-app purchase requirements, external payment restrictions.
    Pay,
    /// Permission declarations, usage descriptions, minimal permission principle.
    Perm,
    /// App descriptions, screenshots, keywords, age ratings.
    Meta,
    /// Private API detection, required reason APIs, deprecated API usage.
    Api,
    /// Data storage, transmission, encryption, retention.
    Data,
    /// ATS compliance, certificate pinning, keychain usage.
    Sec,
    /// Binary size, launch time, background execution limits.
    Perf,
    /// EULA, terms of service, GDPR and CCPA compliance indicators.
    Legal,
    /// `VoiceOver` and `TalkBack` support, dynamic type, contrast ratios.
    Access,
    /// COPPA compliance, kids category requirements, age gating.
    Minor,
    /// Sign in with Apple or Google requirements, account deletion.
    Auth,
    /// Background fetch, push notification usage, network reachability.
    Net,
}

impl Category {
    /// Every category, in taxonomy order.
    pub const ALL: [Category; 14] = [
        Category::Priv,
        Category::Content,
        Category::Pay,
        Category::Perm,
        Category::Meta,
        Category::Api,
        Category::Data,
        Category::Sec,
        Category::Perf,
        Category::Legal,
        Category::Access,
        Category::Minor,
        Category::Auth,
        Category::Net,
    ];

    /// The uppercase code used in rule ids and file names.
    pub fn code(self) -> &'static str {
        match self {
            Category::Priv => "PRIV",
            Category::Content => "CONTENT",
            Category::Pay => "PAY",
            Category::Perm => "PERM",
            Category::Meta => "META",
            Category::Api => "API",
            Category::Data => "DATA",
            Category::Sec => "SEC",
            Category::Perf => "PERF",
            Category::Legal => "LEGAL",
            Category::Access => "ACCESS",
            Category::Minor => "MINOR",
            Category::Auth => "AUTH",
            Category::Net => "NET",
        }
    }

    /// The human readable category name.
    pub fn title(self) -> &'static str {
        match self {
            Category::Priv => "Privacy",
            Category::Content => "Content Policy",
            Category::Pay => "Payments",
            Category::Perm => "Permissions",
            Category::Meta => "Metadata",
            Category::Api => "API Usage",
            Category::Data => "Data Handling",
            Category::Sec => "Security",
            Category::Perf => "Performance",
            Category::Legal => "Legal",
            Category::Access => "Accessibility",
            Category::Minor => "Minors",
            Category::Auth => "Authentication",
            Category::Net => "Networking",
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for Category {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let upper = s.to_ascii_uppercase();
        Category::ALL
            .into_iter()
            .find(|c| c.code() == upper)
            .ok_or_else(|| format!("unknown category code {s:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for category in Category::ALL {
            assert_eq!(Category::from_str(category.code()).unwrap(), category);
        }
    }

    #[test]
    fn codes_are_unique() {
        let mut codes: Vec<&str> = Category::ALL.iter().map(|c| c.code()).collect();
        codes.sort_unstable();
        let count = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), count);
    }

    #[test]
    fn serde_uses_uppercase_codes() {
        let json = serde_json::to_string(&Category::Priv).unwrap();
        assert_eq!(json, "\"PRIV\"");
        let parsed: Category = serde_json::from_str("\"ACCESS\"").unwrap();
        assert_eq!(parsed, Category::Access);
    }
}
