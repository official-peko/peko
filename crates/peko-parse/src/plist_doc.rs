//! Property list parsing. The `plist` crate detects the binary and the XML
//! format on its own, so `Info.plist`, `.entitlements` and
//! `PrivacyInfo.xcprivacy` all use one code path.

use crate::error::{ParseError, Result};
use crate::kind::FileKind;
use crate::value::Document;
use serde_json::{Map, Value};
use std::path::Path;

/// Parse property list bytes into a document.
pub fn parse_plist(kind: FileKind, path: &Path, bytes: &[u8]) -> Result<Document> {
    let parsed = plist::Value::from_reader(std::io::Cursor::new(bytes)).map_err(|source| {
        ParseError::Plist {
            path: path.to_path_buf(),
            source,
        }
    })?;
    let document = Document::new(kind, path, to_json(&parsed));
    // A binary property list holds no text, so it carries no line numbers.
    Ok(match std::str::from_utf8(bytes) {
        Ok(text) => document.with_raw(text),
        Err(_) => document,
    })
}

/// Read and parse a property list file.
pub fn read_plist(kind: FileKind, path: &Path) -> Result<Document> {
    let bytes = std::fs::read(path).map_err(|source| ParseError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_plist(kind, path, &bytes)
}

fn to_json(value: &plist::Value) -> Value {
    match value {
        plist::Value::Array(items) => Value::Array(items.iter().map(to_json).collect()),
        plist::Value::Dictionary(dict) => {
            let mut map = Map::with_capacity(dict.len());
            for (key, item) in dict {
                map.insert(key.clone(), to_json(item));
            }
            Value::Object(map)
        }
        plist::Value::Boolean(b) => Value::Bool(*b),
        plist::Value::Data(bytes) => Value::String(format!("<data: {} bytes>", bytes.len())),
        plist::Value::Date(date) => Value::String(date.to_xml_format()),
        plist::Value::Real(number) => {
            serde_json::Number::from_f64(*number).map_or(Value::Null, Value::Number)
        }
        plist::Value::Integer(number) => number
            .as_signed()
            .map(Value::from)
            .or_else(|| number.as_unsigned().map(Value::from))
            .unwrap_or(Value::Null),
        plist::Value::String(text) => Value::String(text.clone()),
        plist::Value::Uid(uid) => Value::from(uid.get()),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.example.app</string>
    <key>NSCameraUsageDescription</key>
    <string>We use the camera to scan receipts.</string>
    <key>ITSAppUsesNonExemptEncryption</key>
    <false/>
    <key>LSMinimumSystemVersion</key>
    <integer>16</integer>
    <key>UIBackgroundModes</key>
    <array>
        <string>audio</string>
        <string>location</string>
    </array>
</dict>
</plist>
"#;

    #[test]
    fn parses_xml_plist() {
        let doc = parse_plist(
            FileKind::InfoPlist,
            Path::new("Info.plist"),
            SAMPLE.as_bytes(),
        )
        .unwrap();
        assert_eq!(
            doc.lookup_first("CFBundleIdentifier").unwrap().unwrap(),
            &serde_json::json!("com.example.app")
        );
        assert_eq!(
            doc.lookup_first("ITSAppUsesNonExemptEncryption")
                .unwrap()
                .unwrap(),
            &serde_json::json!(false)
        );
        assert_eq!(doc.lookup("UIBackgroundModes[]").unwrap().len(), 2);
        assert_eq!(
            doc.lookup_first("LSMinimumSystemVersion").unwrap().unwrap(),
            &serde_json::json!(16)
        );
    }

    #[test]
    fn parses_binary_plist() {
        let mut binary = Vec::new();
        let value = plist::Value::from_reader(std::io::Cursor::new(SAMPLE.as_bytes())).unwrap();
        value.to_writer_binary(&mut binary).unwrap();
        let doc = parse_plist(FileKind::InfoPlist, Path::new("Info.plist"), &binary).unwrap();
        assert_eq!(
            doc.lookup_first("CFBundleIdentifier").unwrap().unwrap(),
            &serde_json::json!("com.example.app")
        );
    }

    #[test]
    fn reports_a_bad_plist() {
        let err = parse_plist(FileKind::InfoPlist, Path::new("Info.plist"), b"not a plist");
        assert!(err.is_err());
    }
}
