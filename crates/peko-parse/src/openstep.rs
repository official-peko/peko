//! An `OpenStep` property list reader.
//!
//! `project.pbxproj` uses the `OpenStep` format, not the XML format, so the
//! `plist` crate cannot read it. The grammar is small:
//!
//! ```text
//! value      := dict | array | quoted | bare | data
//! dict       := '{' ( key '=' value ';' )* '}'
//! array      := '(' ( value ',' )* value? ')'
//! quoted     := '"' character* '"'
//! bare       := [A-Za-z0-9_./$:@-]+
//! data       := '<' hex* '>'
//! comment    := '/*' character* '*/'  or  '//' to the end of the line
//! ```
//!
//! The reader returns `serde_json::Value`, so the rest of the crate reads a
//! pbxproj with the same key path grammar as every other file.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A problem found while a property list is read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message} at byte {offset}")]
pub struct OpenStepError {
    pub message: String,
    pub offset: usize,
}

type Result<T> = std::result::Result<T, OpenStepError>;

/// Read an `OpenStep` property list.
pub fn parse(text: &str) -> Result<Value> {
    let mut reader = Reader::new(text);
    reader.skip_trivia();

    // A pbxproj opens with `// !$*UTF8*$!` and then one dictionary. A file
    // that opens with a bare `{` reads the same way.
    let value = reader.read_value()?;
    reader.skip_trivia();
    Ok(value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    text: &'a str,
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            text,
            position: 0,
        }
    }

    fn error<T>(&self, message: &str) -> Result<T> {
        Err(OpenStepError {
            message: message.to_string(),
            offset: self.position,
        })
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    /// Step over whitespace and comments.
    fn skip_trivia(&mut self) {
        loop {
            while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
                self.position += 1;
            }
            if self.bytes[self.position..].starts_with(b"/*") {
                if let Some(end) = self.text[self.position + 2..].find("*/") {
                    self.position += 2 + end + 2;
                } else {
                    self.position = self.bytes.len();
                    return;
                }
                continue;
            }
            if self.bytes[self.position..].starts_with(b"//") {
                if let Some(end) = self.text[self.position..].find('\n') {
                    self.position += end + 1;
                } else {
                    self.position = self.bytes.len();
                    return;
                }
                continue;
            }
            return;
        }
    }

    fn read_value(&mut self) -> Result<Value> {
        self.skip_trivia();
        match self.peek() {
            Some(b'{') => self.read_dict(),
            Some(b'(') => self.read_array(),
            Some(b'"') => Ok(Value::String(self.read_quoted()?)),
            Some(b'<') => Ok(Value::String(self.read_data()?)),
            Some(_) => Ok(Value::String(self.read_bare()?)),
            None => self.error("the file ended where a value was expected"),
        }
    }

    fn read_dict(&mut self) -> Result<Value> {
        self.position += 1; // the opening brace
        let mut map = Map::new();

        loop {
            self.skip_trivia();
            match self.peek() {
                Some(b'}') => {
                    self.position += 1;
                    return Ok(Value::Object(map));
                }
                None => return self.error("the file ended inside a dictionary"),
                _ => {}
            }

            let key = match self.peek() {
                Some(b'"') => self.read_quoted()?,
                _ => self.read_bare()?,
            };

            self.skip_trivia();
            if self.peek() != Some(b'=') {
                return self.error("a dictionary key needs an equals sign");
            }
            self.position += 1;

            let value = self.read_value()?;
            map.insert(key, value);

            self.skip_trivia();
            // A semicolon closes an entry. The last entry sometimes omits it.
            if self.peek() == Some(b';') {
                self.position += 1;
            }
        }
    }

    fn read_array(&mut self) -> Result<Value> {
        self.position += 1; // the opening parenthesis
        let mut items = Vec::new();

        loop {
            self.skip_trivia();
            match self.peek() {
                Some(b')') => {
                    self.position += 1;
                    return Ok(Value::Array(items));
                }
                None => return self.error("the file ended inside an array"),
                _ => {}
            }

            items.push(self.read_value()?);
            self.skip_trivia();
            if self.peek() == Some(b',') {
                self.position += 1;
            }
        }
    }

    fn read_quoted(&mut self) -> Result<String> {
        self.position += 1; // the opening quote
        let mut out = String::new();

        loop {
            match self.peek() {
                None => return self.error("the file ended inside a quoted string"),
                Some(b'"') => {
                    self.position += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.position += 1;
                    let escaped = match self.peek() {
                        None => return self.error("the file ended after a backslash"),
                        Some(b'n') => '\n',
                        Some(b't') => '\t',
                        Some(b'r') => '\r',
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(other) => other as char,
                    };
                    out.push(escaped);
                    self.position += 1;
                }
                Some(_) => {
                    // Step by one character, so a multi-byte character stays
                    // whole.
                    let rest = &self.text[self.position..];
                    let character = rest.chars().next().unwrap_or('\u{fffd}');
                    out.push(character);
                    self.position += character.len_utf8();
                }
            }
        }
    }

    fn read_bare(&mut self) -> Result<String> {
        let start = self.position;
        while let Some(byte) = self.peek() {
            let ok = byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'_' | b'.' | b'/' | b'$' | b':' | b'@' | b'-' | b'+' | b'~'
                );
            if !ok {
                break;
            }
            self.position += 1;
        }
        if start == self.position {
            return self.error("a value was expected");
        }
        Ok(self.text[start..self.position].to_string())
    }

    fn read_data(&mut self) -> Result<String> {
        let start = self.position;
        match self.text[self.position..].find('>') {
            Some(end) => {
                self.position += end + 1;
                Ok(self.text[start..self.position].to_string())
            }
            None => self.error("the file ended inside a data block"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_dictionary_reads() {
        let value = parse("{ a = 1; b = two; }").unwrap();
        assert_eq!(value["a"], "1");
        assert_eq!(value["b"], "two");
    }

    #[test]
    fn a_quoted_key_and_value_read() {
        let value = parse(r#"{ "a key" = "a value with spaces"; }"#).unwrap();
        assert_eq!(value["a key"], "a value with spaces");
    }

    #[test]
    fn an_array_reads() {
        let value = parse("{ files = ( one, two, three, ); }").unwrap();
        assert_eq!(value["files"].as_array().unwrap().len(), 3);
        assert_eq!(value["files"][2], "three");
    }

    #[test]
    fn a_nested_dictionary_reads() {
        let value = parse("{ outer = { inner = { deep = yes; }; }; }").unwrap();
        assert_eq!(value["outer"]["inner"]["deep"], "yes");
    }

    #[test]
    fn comments_are_ignored() {
        let text = r"// !$*UTF8*$!
        {
            /* a block comment */
            a = 1; // a line comment
            b /* between */ = 2;
        }";
        let value = parse(text).unwrap();
        assert_eq!(value["a"], "1");
        assert_eq!(value["b"], "2");
    }

    #[test]
    fn an_object_reference_keeps_its_comment_out_of_the_value() {
        // Xcode writes an id and then a comment naming the object.
        let text = "{ target = 1A2B3C /* MyApp */; }";
        let value = parse(text).unwrap();
        assert_eq!(value["target"], "1A2B3C");
    }

    #[test]
    fn an_escape_inside_a_quoted_string_reads() {
        let value = parse(r#"{ a = "line\none \"two\""; }"#).unwrap();
        assert_eq!(value["a"], "line\none \"two\"");
    }

    #[test]
    fn a_build_setting_list_reads() {
        let text = r#"{
            buildSettings = {
                OTHER_LDFLAGS = ( "-ObjC", "-l\"z\"", );
                PRODUCT_BUNDLE_IDENTIFIER = com.example.app;
                INFOPLIST_FILE = "App/Info.plist";
            };
        }"#;
        let value = parse(text).unwrap();
        assert_eq!(
            value["buildSettings"]["PRODUCT_BUNDLE_IDENTIFIER"],
            "com.example.app"
        );
        assert_eq!(value["buildSettings"]["INFOPLIST_FILE"], "App/Info.plist");
        assert_eq!(value["buildSettings"]["OTHER_LDFLAGS"][0], "-ObjC");
    }

    #[test]
    fn a_broken_file_reports_the_offset() {
        let error = parse("{ a = 1;").unwrap_err();
        assert!(error.message.contains("dictionary"), "{error}");
        assert!(error.offset > 0);
    }

    #[test]
    fn a_data_block_reads_as_text() {
        let value = parse("{ a = <0011ff>; }").unwrap();
        assert_eq!(value["a"], "<0011ff>");
    }
}
