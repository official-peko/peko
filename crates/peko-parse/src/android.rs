//! `AndroidManifest.xml` parsing.
//!
//! An element becomes an object. Attributes take an `@` prefix and keep their
//! namespace prefix, for example `@android:name`. A tag that appears once
//! becomes an object. A tag that repeats becomes an array. The key path
//! grammar flattens arrays, so one path form reads both.

use crate::error::{ParseError, Result};
use crate::kind::FileKind;
use crate::value::Document;
use serde_json::{Map, Value};
use std::path::Path;

/// Parse XML text into a document.
pub fn parse_xml(kind: FileKind, path: &Path, text: &str) -> Result<Document> {
    let parsed = roxmltree::Document::parse(text).map_err(|source| ParseError::Xml {
        path: path.to_path_buf(),
        source,
    })?;
    let root = parsed.root_element();
    let mut map = Map::new();
    map.insert(root.tag_name().name().to_string(), element_to_json(root));
    Ok(Document::new(kind, path, Value::Object(map)).with_raw(text))
}

/// Read and parse an XML file.
pub fn read_xml(kind: FileKind, path: &Path) -> Result<Document> {
    let text = std::fs::read_to_string(path).map_err(|source| ParseError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_xml(kind, path, &text)
}

fn element_to_json(node: roxmltree::Node<'_, '_>) -> Value {
    let mut map = Map::new();

    for attribute in node.attributes() {
        let key = match attribute.namespace() {
            Some(uri) => match node.lookup_prefix(uri) {
                Some(prefix) => format!("@{prefix}:{}", attribute.name()),
                None => format!("@{}", attribute.name()),
            },
            None => format!("@{}", attribute.name()),
        };
        map.insert(key, Value::String(attribute.value().to_string()));
    }

    let mut grouped: Vec<(String, Vec<Value>)> = Vec::new();
    for child in node.children().filter(roxmltree::Node::is_element) {
        let name = child.tag_name().name().to_string();
        let value = element_to_json(child);
        match grouped.iter_mut().find(|(key, _)| *key == name) {
            Some((_, values)) => values.push(value),
            None => grouped.push((name, vec![value])),
        }
    }
    for (name, mut values) in grouped {
        if values.len() == 1 {
            map.insert(name, values.remove(0));
        } else {
            map.insert(name, Value::Array(values));
        }
    }

    let text: String = node
        .children()
        .filter(roxmltree::Node::is_text)
        .filter_map(|child| child.text())
        .collect::<Vec<_>>()
        .join("");
    if !text.trim().is_empty() {
        map.insert("#text".to_string(), Value::String(text.trim().to_string()));
    }

    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="com.example.app">
    <uses-permission android:name="android.permission.INTERNET" />
    <uses-permission android:name="android.permission.ACCESS_FINE_LOCATION" />
    <application
        android:allowBackup="true"
        android:usesCleartextTraffic="true"
        android:label="Example">
        <activity android:name=".MainActivity" android:exported="true" />
    </application>
</manifest>
"#;

    fn doc() -> Document {
        parse_xml(
            FileKind::AndroidManifest,
            Path::new("AndroidManifest.xml"),
            MANIFEST,
        )
        .unwrap()
    }

    #[test]
    fn reads_repeated_elements_as_an_array() {
        let d = doc();
        let names = d
            .lookup("manifest.uses-permission[].@android:name")
            .unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&&json!("android.permission.INTERNET")));
        assert!(names.contains(&&json!("android.permission.ACCESS_FINE_LOCATION")));
    }

    #[test]
    fn reads_a_single_element_without_brackets() {
        let d = doc();
        let value = d
            .lookup("manifest.application.@android:allowBackup")
            .unwrap();
        assert_eq!(value, vec![&json!("true")]);
    }

    #[test]
    fn reads_a_nested_element() {
        let d = doc();
        let value = d
            .lookup("manifest.application.activity.@android:exported")
            .unwrap();
        assert_eq!(value, vec![&json!("true")]);
    }

    #[test]
    fn reads_an_attribute_without_a_namespace() {
        let d = doc();
        assert_eq!(
            d.lookup("manifest.@package").unwrap(),
            vec![&json!("com.example.app")]
        );
    }

    #[test]
    fn missing_element_returns_empty() {
        let d = doc();
        assert!(d.lookup("manifest.uses-feature").unwrap().is_empty());
    }

    #[test]
    fn reports_bad_xml() {
        assert!(parse_xml(FileKind::AndroidManifest, Path::new("m.xml"), "<a>").is_err());
    }
}
