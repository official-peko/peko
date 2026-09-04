//! Value matching for manifest keys and build settings.

use peko_parse::display_value;
use peko_rules::ValueMatcher;
use serde_json::Value;

/// What a matcher decided about a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchOutcome {
    /// The value satisfies the matcher.
    Pass,
    /// The value breaks the matcher. The text names what was found.
    Fail(String),
    /// The matcher cannot read the value, so it decides nothing.
    ///
    /// A build script often holds a variable instead of a number, for example
    /// `targetSdk rootProject.targetSdkVersion`. A checker that cannot resolve
    /// the variable knows nothing about the level. Reporting a violation there
    /// is a guess, and a guess on working code costs user trust.
    NotEvaluable(String),
}

impl MatchOutcome {
    pub fn is_pass(&self) -> bool {
        matches!(self, MatchOutcome::Pass)
    }

    /// The failure text, when the value broke the matcher.
    pub fn failure(&self) -> Option<&str> {
        match self {
            MatchOutcome::Fail(reason) => Some(reason),
            _ => None,
        }
    }
}

/// Test a value against a matcher.
pub fn evaluate(matcher: &ValueMatcher, value: &Value) -> MatchOutcome {
    match matcher {
        ValueMatcher::Equals { value: expected } => {
            if compare_loosely(value, expected) {
                MatchOutcome::Pass
            } else {
                MatchOutcome::Fail(format!(
                    "expected {}, found {}",
                    display_value(expected),
                    display_value(value)
                ))
            }
        }
        ValueMatcher::NotEquals { value: forbidden } => {
            if compare_loosely(value, forbidden) {
                MatchOutcome::Fail(format!(
                    "the value must not be {}",
                    display_value(forbidden)
                ))
            } else {
                MatchOutcome::Pass
            }
        }
        ValueMatcher::OneOf { values } => {
            if values
                .iter()
                .any(|candidate| compare_loosely(value, candidate))
            {
                MatchOutcome::Pass
            } else {
                let allowed: Vec<String> = values.iter().map(display_value).collect();
                MatchOutcome::Fail(format!(
                    "expected one of [{}], found {}",
                    allowed.join(", "),
                    display_value(value)
                ))
            }
        }
        ValueMatcher::Regex { pattern } => {
            let text = as_text(value);
            match regex::Regex::new(pattern) {
                Ok(regex) if regex.is_match(&text) => MatchOutcome::Pass,
                Ok(_) => MatchOutcome::Fail(format!(
                    "{} does not match /{pattern}/",
                    display_value(value)
                )),
                Err(error) => MatchOutcome::NotEvaluable(format!(
                    "the rule holds an invalid pattern /{pattern}/: {error}"
                )),
            }
        }
        ValueMatcher::NotRegex { pattern } => {
            let text = as_text(value);
            match regex::Regex::new(pattern) {
                Ok(regex) if regex.is_match(&text) => MatchOutcome::Fail(format!(
                    "{} must not match /{pattern}/",
                    display_value(value)
                )),
                Ok(_) => MatchOutcome::Pass,
                Err(error) => MatchOutcome::NotEvaluable(format!(
                    "the rule holds an invalid pattern /{pattern}/: {error}"
                )),
            }
        }
        ValueMatcher::MinInt { value: minimum } => match as_integer(value) {
            Some(found) if found >= *minimum => MatchOutcome::Pass,
            Some(found) => {
                MatchOutcome::Fail(format!("expected at least {minimum}, found {found}"))
            }
            None => MatchOutcome::NotEvaluable(format!(
                "{} is not a number that this checker can read",
                display_value(value)
            )),
        },
        ValueMatcher::MaxInt { value: maximum } => match as_integer(value) {
            Some(found) if found <= *maximum => MatchOutcome::Pass,
            Some(found) => MatchOutcome::Fail(format!("expected at most {maximum}, found {found}")),
            None => MatchOutcome::NotEvaluable(format!(
                "{} is not a number that this checker can read",
                display_value(value)
            )),
        },
        ValueMatcher::NonEmptyString { min_length } => match value {
            Value::String(text) => {
                let length = text.trim().chars().count();
                if length >= *min_length {
                    MatchOutcome::Pass
                } else if length == 0 {
                    MatchOutcome::Fail("the value is an empty string".to_string())
                } else {
                    MatchOutcome::Fail(format!(
                        "the value holds {length} characters, the rule requires at least {min_length}"
                    ))
                }
            }
            other => MatchOutcome::NotEvaluable(format!(
                "expected a string, found {}",
                display_value(other)
            )),
        },
    }
}

/// True when a build setting holds an unresolved reference instead of a value.
///
/// A build script often names a variable, for example
/// `targetSdkVersion rootProject.targetSdkVersion` or `targetSdk target_sdk`.
/// This checker does not evaluate a build script, so it cannot read the value
/// behind the name.
pub fn is_unresolved_reference(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains('$') {
        return true;
    }
    if trimmed.parse::<i64>().is_ok() || trimmed.parse::<f64>().is_ok() {
        return false;
    }
    if matches!(
        trimmed.to_ascii_lowercase().as_str(),
        "true" | "false" | "yes" | "no"
    ) {
        return false;
    }
    // A bare identifier, or a dotted path of identifiers, names a variable.
    trimmed.split('.').all(|part| {
        !part.is_empty()
            && part.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Compare a parsed value with an expected value.
///
/// XML attributes and build settings are always text. `"true"` therefore has
/// to equal `true`, and `"35"` has to equal `35`.
fn compare_loosely(found: &Value, expected: &Value) -> bool {
    if found == expected {
        return true;
    }
    match (found, expected) {
        (Value::String(text), Value::Bool(flag)) | (Value::Bool(flag), Value::String(text)) => {
            matches!(
                (text.to_ascii_lowercase().as_str(), flag),
                ("true" | "yes", true) | ("false" | "no", false)
            )
        }
        (Value::String(text), Value::Number(number))
        | (Value::Number(number), Value::String(text)) => text.trim() == number.to_string(),
        _ => false,
    }
}

fn as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn as_integer(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64().or_else(|| {
            // A version such as 16.0 truncates toward zero, so "16.5" reads as
            // 16. A minimum check must not pass on a fractional part alone.
            #[allow(clippy::cast_possible_truncation)]
            number.as_f64().map(|value| value as i64)
        }),
        Value::String(text) => {
            let trimmed = text.trim();
            trimmed
                .parse::<i64>()
                .ok()
                .or_else(|| trimmed.split('.').next()?.parse::<i64>().ok())
        }
        Value::Bool(flag) => Some(i64::from(*flag)),
        _ => None,
    }
}

/// Turn a build setting string into a value. Numbers and booleans keep their
/// type so that a matcher reads them correctly.
pub fn setting_to_value(text: &str) -> Value {
    let trimmed = text.trim();
    if let Ok(number) = trimmed.parse::<i64>() {
        return Value::from(number);
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "true" | "yes" => Value::Bool(true),
        "false" | "no" => Value::Bool(false),
        _ => Value::String(trimmed.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn equals_compares_text_with_booleans() {
        let matcher = ValueMatcher::Equals {
            value: json!(false),
        };
        assert!(evaluate(&matcher, &json!("false")).is_pass());
        assert!(evaluate(&matcher, &json!(false)).is_pass());
        assert!(!evaluate(&matcher, &json!("true")).is_pass());
    }

    #[test]
    fn min_int_reads_text_versions() {
        let matcher = ValueMatcher::MinInt { value: 35 };
        assert!(evaluate(&matcher, &json!("35")).is_pass());
        assert!(evaluate(&matcher, &json!(36)).is_pass());
        let failure = evaluate(&matcher, &json!("34"));
        assert!(
            failure.failure().unwrap().contains("expected at least 35"),
            "{failure:?}"
        );
    }

    #[test]
    fn min_int_reads_a_dotted_deployment_target() {
        let matcher = ValueMatcher::MinInt { value: 16 };
        assert!(evaluate(&matcher, &json!("16.0")).is_pass());
        assert!(!evaluate(&matcher, &json!("15.5")).is_pass());
    }

    #[test]
    fn non_empty_string_rejects_blank_usage_descriptions() {
        let matcher = ValueMatcher::NonEmptyString { min_length: 10 };
        assert!(evaluate(&matcher, &json!("We scan receipts with the camera.")).is_pass());
        assert!(evaluate(&matcher, &json!("   ")).failure().is_some());
        assert!(evaluate(&matcher, &json!("camera")).failure().is_some());
        assert!(matches!(
            evaluate(&matcher, &json!(42)),
            MatchOutcome::NotEvaluable(_)
        ));
    }

    #[test]
    fn one_of_lists_the_allowed_values() {
        let matcher = ValueMatcher::OneOf {
            values: vec![json!("CA92.1"), json!("1C8F.1")],
        };
        assert!(evaluate(&matcher, &json!("CA92.1")).is_pass());
        let failure = evaluate(&matcher, &json!("XXXX.1"));
        assert!(failure.failure().unwrap().contains("CA92.1"), "{failure:?}");
    }

    #[test]
    fn not_regex_rejects_restricted_values() {
        let matcher = ValueMatcher::NotRegex {
            pattern: "^android\\.permission\\.(READ_SMS|READ_CALL_LOG)$".into(),
        };
        assert!(evaluate(&matcher, &json!("android.permission.INTERNET")).is_pass());
        assert!(evaluate(&matcher, &json!("android.permission.READ_SMS"))
            .failure()
            .is_some());
    }

    #[test]
    fn a_variable_reference_decides_nothing() {
        let matcher = ValueMatcher::MinInt { value: 35 };
        // A real build script from the validation corpus.
        for reference in ["rootProject.targetSdkVersion", "target_sdk", "${targetSdk}"] {
            assert!(
                matches!(
                    evaluate(&matcher, &json!(reference)),
                    MatchOutcome::NotEvaluable(_)
                ),
                "{reference} must decide nothing"
            );
            assert!(is_unresolved_reference(reference), "{reference}");
        }
    }

    #[test]
    fn a_literal_is_resolved() {
        for literal in ["35", "16.0", "true", "false"] {
            assert!(!is_unresolved_reference(literal), "{literal}");
        }
    }

    #[test]
    fn setting_values_keep_their_type() {
        assert_eq!(setting_to_value("35"), json!(35));
        assert_eq!(setting_to_value("true"), json!(true));
        assert_eq!(
            setting_to_value("com.example.app"),
            json!("com.example.app")
        );
    }
}
