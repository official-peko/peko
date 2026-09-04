//! A common key-value document model and its key path grammar.
//!
//! Every manifest style file becomes a `serde_json::Value`. One path grammar
//! then addresses property lists and XML alike.
//!
//! # Key path grammar
//!
//! | Form | Meaning |
//! |---|---|
//! | `Key` | A dictionary key or an XML child element |
//! | `@name` | An XML attribute |
//! | `[]` | Every element of an array |
//! | `[2]` | One element of an array by index |
//! | `['a.b']` | A literal key that contains dots |
//!
//! Arrays flatten on their own. `NSPrivacyAccessedAPITypes.NSPrivacyAccessedAPIType`
//! and `NSPrivacyAccessedAPITypes[].NSPrivacyAccessedAPIType` return the same
//! values. Write `[]` when it helps a reader.

use crate::error::{ParseError, Result};
use crate::kind::FileKind;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// One step of a key path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// A dictionary key or an XML element name.
    Key(String),
    /// An XML attribute name, written `@name`.
    Attr(String),
    /// One array element by index.
    Index(usize),
    /// Every array element.
    Wildcard,
}

/// A parsed key path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPath {
    segments: Vec<Segment>,
    raw: String,
}

impl KeyPath {
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Parse a key path.
    #[allow(unused_assignments)]
    pub fn parse(path: &str) -> Result<Self> {
        let invalid = |reason: &str| ParseError::InvalidKeyPath {
            path: path.to_string(),
            reason: reason.to_string(),
        };
        if path.trim().is_empty() {
            return Err(invalid("a key path must not be empty"));
        }

        let mut segments = Vec::new();
        let chars: Vec<char> = path.chars().collect();
        let mut position = 0usize;
        let mut current = String::new();
        let mut is_attr = false;
        let mut have_word = false;

        // Close the identifier collected so far, if any.
        macro_rules! flush {
            () => {
                if have_word {
                    if current.is_empty() {
                        return Err(invalid("empty segment"));
                    }
                    if is_attr {
                        segments.push(Segment::Attr(std::mem::take(&mut current)));
                    } else {
                        segments.push(Segment::Key(std::mem::take(&mut current)));
                    }
                    is_attr = false;
                    have_word = false;
                }
            };
        }

        while position < chars.len() {
            match chars[position] {
                '.' => {
                    flush!();
                    if position + 1 == chars.len() {
                        return Err(invalid("a key path must not end with a dot"));
                    }
                    position += 1;
                }
                '@' if !have_word => {
                    is_attr = true;
                    have_word = true;
                    position += 1;
                }
                '[' => {
                    flush!();
                    let close = chars[position..]
                        .iter()
                        .position(|c| *c == ']')
                        .ok_or_else(|| invalid("unclosed bracket"))?
                        + position;
                    let inner: String = chars[position + 1..close].iter().collect();
                    if inner.is_empty() {
                        segments.push(Segment::Wildcard);
                    } else if inner.len() >= 2 && inner.starts_with('\'') && inner.ends_with('\'') {
                        let literal = inner[1..inner.len() - 1].to_string();
                        if literal.is_empty() {
                            return Err(invalid("empty quoted key"));
                        }
                        segments.push(Segment::Key(literal));
                    } else {
                        let index: usize = inner.parse().map_err(|_| {
                            invalid("bracket must hold an index, a quoted key, or nothing")
                        })?;
                        segments.push(Segment::Index(index));
                    }
                    position = close + 1;
                }
                other => {
                    current.push(other);
                    have_word = true;
                    position += 1;
                }
            }
        }
        flush!();

        if segments.is_empty() {
            return Err(invalid("a key path must hold at least one segment"));
        }
        Ok(Self {
            segments,
            raw: path.to_string(),
        })
    }
}

/// A parsed manifest style file.
#[derive(Debug, Clone)]
pub struct Document {
    kind: FileKind,
    path: PathBuf,
    root: Value,
    /// The original text, when the file is text. A binary property list has
    /// none. Findings use it to name a line.
    raw: Option<String>,
}

impl Document {
    pub fn new(kind: FileKind, path: impl Into<PathBuf>, root: Value) -> Self {
        Self {
            kind,
            path: path.into(),
            root,
            raw: None,
        }
    }

    /// Attach the original text so that a finding can name a line.
    #[must_use]
    pub fn with_raw(mut self, raw: impl Into<String>) -> Self {
        self.raw = Some(raw.into());
        self
    }

    /// The one-based line that first holds `needle`.
    ///
    /// A manifest parser produces a tree with no positions. A finding names
    /// the value it objects to, and the value is unique in almost every real
    /// manifest, so a text search names the right line. A miss returns `None`,
    /// and the finding then names the file alone.
    pub fn line_of(&self, needle: &str) -> Option<usize> {
        if needle.trim().is_empty() {
            return None;
        }
        let raw = self.raw.as_ref()?;
        let offset = raw.find(needle)?;
        Some(raw[..offset].matches('\n').count() + 1)
    }

    /// The text of one line, without the line break.
    pub fn line_text(&self, line: usize) -> Option<&str> {
        self.raw.as_ref()?.lines().nth(line.checked_sub(1)?)
    }

    pub fn kind(&self) -> FileKind {
        self.kind
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn root(&self) -> &Value {
        &self.root
    }

    /// Every value that the key path addresses. An empty result means the key
    /// is absent.
    pub fn lookup(&self, path: &str) -> Result<Vec<&Value>> {
        let parsed = KeyPath::parse(path)?;
        Ok(self.lookup_parsed(&parsed))
    }

    /// Every value that a parsed key path addresses.
    pub fn lookup_parsed(&self, path: &KeyPath) -> Vec<&Value> {
        let mut current: Vec<&Value> = vec![&self.root];
        for segment in path.segments() {
            current = step(&current, segment);
            if current.is_empty() {
                break;
            }
        }
        current
    }

    /// The first value the key path addresses.
    pub fn lookup_first(&self, path: &str) -> Result<Option<&Value>> {
        Ok(self.lookup(path)?.into_iter().next())
    }

    /// True if the key path addresses at least one value.
    pub fn exists(&self, path: &str) -> Result<bool> {
        Ok(!self.lookup(path)?.is_empty())
    }

    /// Look up one top level key without applying the path grammar. Use this
    /// for entitlement keys, which contain dots.
    pub fn get_literal(&self, key: &str) -> Option<&Value> {
        self.root.as_object()?.get(key)
    }

    /// Every top level key. Empty when the root is not a dictionary.
    pub fn top_level_keys(&self) -> Vec<&str> {
        self.root
            .as_object()
            .map(|map| map.keys().map(String::as_str).collect())
            .unwrap_or_default()
    }
}

fn step<'a>(values: &[&'a Value], segment: &Segment) -> Vec<&'a Value> {
    let mut out = Vec::new();
    match segment {
        Segment::Key(key) => {
            for value in flatten(values) {
                if let Some(found) = value.as_object().and_then(|map| map.get(key)) {
                    out.push(found);
                }
            }
        }
        Segment::Attr(name) => {
            let key = format!("@{name}");
            for value in flatten(values) {
                if let Some(found) = value.as_object().and_then(|map| map.get(&key)) {
                    out.push(found);
                }
            }
        }
        Segment::Index(index) => {
            for value in values {
                if let Some(found) = value.as_array().and_then(|items| items.get(*index)) {
                    out.push(found);
                }
            }
        }
        Segment::Wildcard => {
            for value in values {
                match value.as_array() {
                    Some(items) => out.extend(items.iter()),
                    None => out.push(value),
                }
            }
        }
    }
    out
}

/// Expand arrays so that a key segment reaches the objects inside them.
fn flatten<'a>(values: &[&'a Value]) -> Vec<&'a Value> {
    let mut out = Vec::new();
    for value in values {
        match value {
            Value::Array(items) => {
                let refs: Vec<&Value> = items.iter().collect();
                out.extend(flatten(&refs));
            }
            other => out.push(*other),
        }
    }
    out
}

/// Render a value for a finding message. Long values are cut short.
pub fn display_value(value: &Value) -> String {
    let text = match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if text.chars().count() > 120 {
        let cut: String = text.chars().take(117).collect();
        format!("{cut}...")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc() -> Document {
        Document::new(
            FileKind::PrivacyManifest,
            "PrivacyInfo.xcprivacy",
            json!({
                "NSPrivacyTracking": false,
                "NSPrivacyAccessedAPITypes": [
                    {
                        "NSPrivacyAccessedAPIType": "NSPrivacyAccessedAPICategoryUserDefaults",
                        "NSPrivacyAccessedAPITypeReasons": ["CA92.1"]
                    },
                    {
                        "NSPrivacyAccessedAPIType": "NSPrivacyAccessedAPICategoryFileTimestamp",
                        "NSPrivacyAccessedAPITypeReasons": ["C617.1"]
                    }
                ],
                "manifest": [{ "@android:allowBackup": "true" }],
                "com.apple.developer.healthkit": true
            }),
        )
    }

    #[test]
    fn reads_a_plain_key() {
        let d = doc();
        assert_eq!(d.lookup("NSPrivacyTracking").unwrap(), vec![&json!(false)]);
    }

    #[test]
    fn flattens_arrays_without_explicit_brackets() {
        let d = doc();
        let with = d
            .lookup("NSPrivacyAccessedAPITypes[].NSPrivacyAccessedAPIType")
            .unwrap();
        let without = d
            .lookup("NSPrivacyAccessedAPITypes.NSPrivacyAccessedAPIType")
            .unwrap();
        assert_eq!(with, without);
        assert_eq!(with.len(), 2);
    }

    #[test]
    fn reads_an_index_and_an_attribute() {
        let d = doc();
        let value = d
            .lookup("NSPrivacyAccessedAPITypes[1].NSPrivacyAccessedAPIType")
            .unwrap();
        assert_eq!(
            value,
            vec![&json!("NSPrivacyAccessedAPICategoryFileTimestamp")]
        );
        let attr = d.lookup("manifest.@android:allowBackup").unwrap();
        assert_eq!(attr, vec![&json!("true")]);
    }

    #[test]
    fn reads_a_quoted_literal_key() {
        let d = doc();
        assert_eq!(
            d.lookup("['com.apple.developer.healthkit']").unwrap(),
            vec![&json!(true)]
        );
        assert_eq!(
            d.get_literal("com.apple.developer.healthkit"),
            Some(&json!(true))
        );
    }

    #[test]
    fn missing_keys_return_empty() {
        let d = doc();
        assert!(d.lookup("NSPrivacyNothing").unwrap().is_empty());
        assert!(!d.exists("NSPrivacyNothing").unwrap());
    }

    #[test]
    fn rejects_malformed_paths() {
        for bad in ["", "a.", "a[", "a[x]", "a.[]b.", "['']"] {
            assert!(KeyPath::parse(bad).is_err(), "{bad:?} must not parse");
        }
    }
}
