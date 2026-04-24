//! MCP elicitation-input validation.
//!
//! Port of `src/utils/mcp/elicitationValidation.ts`. When an MCP server
//! elicits input from the user, the returned `requestedSchema` narrows
//! the acceptable values (string formats, number ranges, enum
//! alternatives). This module validates a user-typed string against
//! that schema and produces a normalised primitive value.
//!
//! Scope: synchronous validation only. The TS async path uses a Haiku
//! call to parse natural-language dates ("tomorrow at 3pm") — that's
//! deferred until we wire an NL-datetime helper on top of the secondary
//! model.

use std::fmt;

/// Supported primitive schema shapes, matching TS
/// `PrimitiveSchemaDefinition`. Only the fields the validator actually
/// looks at are carried.
#[derive(Debug, Clone, PartialEq)]
pub enum PrimitiveSchema {
    /// Plain string with optional length + format constraints.
    String {
        min_length: Option<usize>,
        max_length: Option<usize>,
        format: Option<StringFormat>,
    },
    /// Number (floating point). `integer` variant below enforces int.
    Number {
        minimum: Option<f64>,
        maximum: Option<f64>,
    },
    Integer {
        minimum: Option<i64>,
        maximum: Option<i64>,
    },
    Boolean,
    /// Single-select enum: `{"type":"string", "enum":[...]}` or oneOf.
    Enum {
        values: Vec<String>,
        labels: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringFormat {
    Email,
    Uri,
    Date,
    DateTime,
}

impl fmt::Display for StringFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                StringFormat::Email => "email",
                StringFormat::Uri => "uri",
                StringFormat::Date => "date",
                StringFormat::DateTime => "date-time",
            }
        )
    }
}

/// The normalised result of a successful validation.
#[derive(Debug, Clone, PartialEq)]
pub enum Primitive {
    String(String),
    Number(f64),
    Integer(i64),
    Boolean(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub value: Option<Primitive>,
    pub error: Option<String>,
}

impl ValidationResult {
    fn ok(p: Primitive) -> Self {
        Self {
            is_valid: true,
            value: Some(p),
            error: None,
        }
    }
    fn err(msg: impl Into<String>) -> Self {
        Self {
            is_valid: false,
            value: None,
            error: Some(msg.into()),
        }
    }
}

/// Pluralise a unit word based on count. Mirrors TS stringUtils.plural.
fn plural(n: usize, unit: &str) -> String {
    if n == 1 {
        unit.to_string()
    } else {
        format!("{}s", unit)
    }
}

/// Validate `input` against `schema`. Unicode-aware length counting
/// matches TS z.string().min/max behaviour (code-point count).
pub fn validate_elicitation_input(input: &str, schema: &PrimitiveSchema) -> ValidationResult {
    match schema {
        PrimitiveSchema::String {
            min_length,
            max_length,
            format,
        } => validate_string(input, *min_length, *max_length, *format),
        PrimitiveSchema::Number { minimum, maximum } => {
            validate_number(input, *minimum, *maximum, false)
        }
        PrimitiveSchema::Integer { minimum, maximum } => {
            let min = minimum.map(|v| v as f64);
            let max = maximum.map(|v| v as f64);
            validate_number(input, min, max, true)
        }
        PrimitiveSchema::Boolean => validate_boolean(input),
        PrimitiveSchema::Enum { values, .. } => {
            if values.iter().any(|v| v == input) {
                ValidationResult::ok(Primitive::String(input.to_string()))
            } else {
                ValidationResult::err(format!("Must be one of: {}", values.join(", ")))
            }
        }
    }
}

fn validate_string(
    input: &str,
    min_length: Option<usize>,
    max_length: Option<usize>,
    format: Option<StringFormat>,
) -> ValidationResult {
    let char_count = input.chars().count();
    if let Some(min) = min_length {
        if char_count < min {
            return ValidationResult::err(format!(
                "Must be at least {} {}",
                min,
                plural(min, "character")
            ));
        }
    }
    if let Some(max) = max_length {
        if char_count > max {
            return ValidationResult::err(format!(
                "Must be at most {} {}",
                max,
                plural(max, "character")
            ));
        }
    }
    if let Some(f) = format {
        if !passes_format(input, f) {
            return ValidationResult::err(format_error_message(f));
        }
    }
    ValidationResult::ok(Primitive::String(input.to_string()))
}

fn passes_format(input: &str, f: StringFormat) -> bool {
    match f {
        StringFormat::Email => is_email(input),
        StringFormat::Uri => is_uri(input),
        StringFormat::Date => is_iso_date(input),
        StringFormat::DateTime => is_iso_date_time(input),
    }
}

fn format_error_message(f: StringFormat) -> String {
    match f {
        StringFormat::Email => "Must be a valid email address, e.g. user@example.com".into(),
        StringFormat::Uri => "Must be a valid URI, e.g. https://example.com".into(),
        StringFormat::Date => "Must be a valid date, e.g. 2024-03-15, today, next Monday".into(),
        StringFormat::DateTime => {
            "Must be a valid date-time, e.g. 2024-03-15T14:30:00Z, tomorrow at 3pm".into()
        }
    }
}

fn is_email(s: &str) -> bool {
    // Minimal sanity check — local@domain.tld with no whitespace.
    let at = match s.find('@') {
        Some(i) => i,
        None => return false,
    };
    let (local, rest) = s.split_at(at);
    let domain = &rest[1..];
    if local.is_empty() || domain.is_empty() || s.contains(char::is_whitespace) {
        return false;
    }
    domain.contains('.')
}

fn is_uri(s: &str) -> bool {
    // Require scheme://rest.
    if let Some(idx) = s.find("://") {
        let scheme = &s[..idx];
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
            && s.len() > idx + 3
    } else {
        false
    }
}

fn is_iso_date(s: &str) -> bool {
    // YYYY-MM-DD
    if s.len() != 10 {
        return false;
    }
    let bytes = s.as_bytes();
    bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

fn is_iso_date_time(s: &str) -> bool {
    // Minimal ISO 8601 check: YYYY-MM-DDTHH:MM:SS with optional fractional +
    // timezone indicator (Z or ±HH:MM). Not a full parser — good enough for
    // elicitation input where a bad value just retries.
    if s.len() < 19 {
        return false;
    }
    let bytes = s.as_bytes();
    if !is_iso_date(&s[..10]) {
        return false;
    }
    if bytes[10] != b'T' {
        return false;
    }
    bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[11..13].iter().all(u8::is_ascii_digit)
        && bytes[14..16].iter().all(u8::is_ascii_digit)
        && bytes[17..19].iter().all(u8::is_ascii_digit)
}

fn validate_number(
    input: &str,
    min: Option<f64>,
    max: Option<f64>,
    require_int: bool,
) -> ValidationResult {
    let type_label = if require_int {
        "an integer"
    } else {
        "a number"
    };
    let range_msg = match (min, max) {
        (Some(lo), Some(hi)) => format!(
            "Must be {} between {} and {}",
            type_label,
            format_num(lo, require_int),
            format_num(hi, require_int)
        ),
        (Some(lo), None) => format!("Must be {} >= {}", type_label, format_num(lo, require_int)),
        (None, Some(hi)) => format!("Must be {} <= {}", type_label, format_num(hi, require_int)),
        (None, None) => format!("Must be {}", type_label),
    };

    let parsed: f64 = match input.trim().parse() {
        Ok(n) => n,
        Err(_) => return ValidationResult::err(range_msg),
    };
    if require_int && parsed.fract() != 0.0 {
        return ValidationResult::err(range_msg);
    }
    if let Some(lo) = min {
        if parsed < lo {
            return ValidationResult::err(range_msg);
        }
    }
    if let Some(hi) = max {
        if parsed > hi {
            return ValidationResult::err(range_msg);
        }
    }
    if require_int {
        ValidationResult::ok(Primitive::Integer(parsed as i64))
    } else {
        ValidationResult::ok(Primitive::Number(parsed))
    }
}

fn format_num(n: f64, is_integer: bool) -> String {
    if !is_integer && n.fract() == 0.0 {
        format!("{:.1}", n) // "3.0" — matches TS `${n}.0`
    } else {
        // Trim trailing .0 for integers.
        let s = format!("{}", n);
        s
    }
}

fn validate_boolean(input: &str) -> ValidationResult {
    match input.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "y" | "on" => ValidationResult::ok(Primitive::Boolean(true)),
        "false" | "0" | "no" | "n" | "off" => ValidationResult::ok(Primitive::Boolean(false)),
        _ => ValidationResult::err("Must be true or false"),
    }
}

/// Return a placeholder/hint for UIs rendering the input prompt.
/// Mirrors TS `getFormatHint`.
pub fn get_format_hint(schema: &PrimitiveSchema) -> Option<String> {
    match schema {
        PrimitiveSchema::String {
            format: Some(StringFormat::Email),
            ..
        } => Some("email address, e.g. user@example.com".into()),
        PrimitiveSchema::String {
            format: Some(StringFormat::Uri),
            ..
        } => Some("URI, e.g. https://example.com".into()),
        PrimitiveSchema::String {
            format: Some(StringFormat::Date),
            ..
        } => Some("date, e.g. 2024-03-15".into()),
        PrimitiveSchema::String {
            format: Some(StringFormat::DateTime),
            ..
        } => Some("date-time, e.g. 2024-03-15T14:30:00Z".into()),
        PrimitiveSchema::Number { minimum, maximum } => Some(number_hint(
            "number",
            minimum.map(|v| v),
            maximum.map(|v| v),
            false,
        )),
        PrimitiveSchema::Integer { minimum, maximum } => Some(number_hint(
            "integer",
            minimum.map(|v| v as f64),
            maximum.map(|v| v as f64),
            true,
        )),
        _ => None,
    }
}

fn number_hint(type_name: &str, min: Option<f64>, max: Option<f64>, is_integer: bool) -> String {
    match (min, max) {
        (Some(lo), Some(hi)) => format!(
            "({} between {} and {})",
            type_name,
            format_num(lo, is_integer),
            format_num(hi, is_integer)
        ),
        (Some(lo), None) => format!("({} >= {})", type_name, format_num(lo, is_integer)),
        (None, Some(hi)) => format!("({} <= {})", type_name, format_num(hi, is_integer)),
        (None, None) => {
            let example = if is_integer { "42" } else { "3.14" };
            format!("({}, e.g. {})", type_name, example)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_length_bounds() {
        let schema = PrimitiveSchema::String {
            min_length: Some(3),
            max_length: Some(5),
            format: None,
        };
        assert!(!validate_elicitation_input("ab", &schema).is_valid);
        assert!(validate_elicitation_input("abc", &schema).is_valid);
        assert!(!validate_elicitation_input("abcdef", &schema).is_valid);
    }

    #[test]
    fn email_format_validates() {
        let schema = PrimitiveSchema::String {
            min_length: None,
            max_length: None,
            format: Some(StringFormat::Email),
        };
        assert!(validate_elicitation_input("user@example.com", &schema).is_valid);
        assert!(!validate_elicitation_input("not an email", &schema).is_valid);
        assert!(!validate_elicitation_input("foo@", &schema).is_valid);
    }

    #[test]
    fn uri_format_validates() {
        let schema = PrimitiveSchema::String {
            min_length: None,
            max_length: None,
            format: Some(StringFormat::Uri),
        };
        assert!(validate_elicitation_input("https://example.com", &schema).is_valid);
        assert!(!validate_elicitation_input("example.com", &schema).is_valid);
    }

    #[test]
    fn date_format_validates() {
        let schema = PrimitiveSchema::String {
            min_length: None,
            max_length: None,
            format: Some(StringFormat::Date),
        };
        assert!(validate_elicitation_input("2024-03-15", &schema).is_valid);
        assert!(!validate_elicitation_input("March 15", &schema).is_valid);
    }

    #[test]
    fn integer_range_enforced() {
        let schema = PrimitiveSchema::Integer {
            minimum: Some(1),
            maximum: Some(10),
        };
        assert!(validate_elicitation_input("5", &schema).is_valid);
        let r = validate_elicitation_input("0", &schema);
        assert!(!r.is_valid);
        assert!(r.error.unwrap().contains("between 1 and 10"));
        assert!(!validate_elicitation_input("5.5", &schema).is_valid);
    }

    #[test]
    fn number_range_inclusive() {
        let schema = PrimitiveSchema::Number {
            minimum: Some(0.0),
            maximum: Some(1.0),
        };
        assert!(validate_elicitation_input("0", &schema).is_valid);
        assert!(validate_elicitation_input("1", &schema).is_valid);
        assert!(validate_elicitation_input("0.5", &schema).is_valid);
        assert!(!validate_elicitation_input("1.1", &schema).is_valid);
    }

    #[test]
    fn boolean_accepts_common_forms() {
        let schema = PrimitiveSchema::Boolean;
        assert_eq!(
            validate_elicitation_input("yes", &schema).value,
            Some(Primitive::Boolean(true))
        );
        assert_eq!(
            validate_elicitation_input("FALSE", &schema).value,
            Some(Primitive::Boolean(false))
        );
        assert!(!validate_elicitation_input("maybe", &schema).is_valid);
    }

    #[test]
    fn enum_constrains_values() {
        let schema = PrimitiveSchema::Enum {
            values: vec!["a".into(), "b".into()],
            labels: vec!["Alpha".into(), "Bravo".into()],
        };
        assert!(validate_elicitation_input("a", &schema).is_valid);
        assert!(!validate_elicitation_input("c", &schema).is_valid);
    }

    #[test]
    fn format_hint_for_email() {
        let schema = PrimitiveSchema::String {
            min_length: None,
            max_length: None,
            format: Some(StringFormat::Email),
        };
        assert_eq!(
            get_format_hint(&schema).as_deref(),
            Some("email address, e.g. user@example.com")
        );
    }

    #[test]
    fn format_hint_for_integer_range() {
        let schema = PrimitiveSchema::Integer {
            minimum: Some(1),
            maximum: Some(100),
        };
        assert_eq!(
            get_format_hint(&schema).as_deref(),
            Some("(integer between 1 and 100)")
        );
    }
}
