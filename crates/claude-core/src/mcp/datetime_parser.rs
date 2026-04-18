//! Natural-language date/time parser used by MCP elicitation.
//!
//! Port of `src/utils/mcp/dateTimeParser.ts`. The TS version asks Haiku
//! to convert strings like "tomorrow at 3pm" into ISO 8601. This module
//! ports:
//!   - `looks_like_iso8601(input)` — cheap classifier that decides
//!     whether the elicitation layer should attempt NL parsing or
//!     treat the input as already-formatted.
//!   - `DateTimeFormat`, `DateTimeParseResult` shapes so callers can
//!     thread the parse decision through without stringly-typed enums.
//!   - `build_datetime_parser_prompt(input, format)` — produces the
//!     exact Haiku prompt the TS version sends. Kept verbatim so
//!     prompt-cache hits span TS and Rust clients. The actual LLM call
//!     is deferred until we wire a Haiku helper on top of the
//!     secondary_model trait (current trait is single-prompt / single-
//!     response, which is all the TS call needs).

use chrono::{Datelike, Local, Offset, TimeZone, Timelike};

/// Format of the expected ISO 8601 output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTimeFormat {
    Date,
    DateTime,
}

impl DateTimeFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            DateTimeFormat::Date => "date",
            DateTimeFormat::DateTime => "date-time",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateTimeParseResult {
    Ok(String),
    Err(String),
}

/// Does `input` already look like ISO 8601? Matches TS regex:
/// `^\d{4}-\d{2}-\d{2}(T|$)`. Used to skip the Haiku call when the
/// user typed a proper ISO string.
pub fn looks_like_iso_8601(input: &str) -> bool {
    let s = input.trim();
    if s.len() < 10 {
        return false;
    }
    let bytes = s.as_bytes();
    bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
        && (s.len() == 10 || bytes[10] == b'T')
}

/// Build the Haiku user prompt used to convert natural-language input
/// into ISO 8601. Matches TS text verbatim — modulo the timezone +
/// current-time which we compute from chrono::Local instead of Date.
pub fn build_datetime_parser_prompt(input: &str, format: DateTimeFormat) -> String {
    let now = Local::now();
    let current_datetime = now.with_timezone(&chrono::Utc).to_rfc3339();
    let day_of_week = weekday_name(now.weekday());
    let timezone = format_timezone(&now);

    let format_description = match format {
        DateTimeFormat::Date => "YYYY-MM-DD (date only, no time)".to_string(),
        DateTimeFormat::DateTime => format!(
            "YYYY-MM-DDTHH:MM:SS{} (full date-time with timezone)",
            timezone
        ),
    };

    format!(
        "Current context:\n\
         - Current date and time: {current_datetime} (UTC)\n\
         - Local timezone: {timezone}\n\
         - Day of week: {day_of_week}\n\n\
         User input: \"{input}\"\n\n\
         Output format: {format_description}\n\n\
         Parse the user's input into ISO 8601 format. Return ONLY the formatted string, or \"INVALID\" if the input is incomplete or unparseable."
    )
}

/// System prompt for the parser. Verbatim from TS so cache hits cross
/// implementations.
pub const DATETIME_PARSER_SYSTEM_PROMPT: &str = "You are a date/time parser that converts natural language into ISO 8601 format.\n\
You MUST respond with ONLY the ISO 8601 formatted string, with no explanation or additional text.\n\
If the input is ambiguous, prefer future dates over past dates.\n\
For times without dates, use today's date.\n\
For dates without times, do not include a time component.\n\
If the input is incomplete or you cannot confidently parse it into a valid date, respond with exactly \"INVALID\" (nothing else).\n\
Examples of INVALID input: partial dates like \"2025-01-\", lone numbers like \"13\", gibberish.\n\
Examples of valid natural language: \"tomorrow\", \"next Monday\", \"jan 1st 2025\", \"in 2 hours\", \"yesterday\".";

/// Post-process the model's reply. Returns Ok(value) when the reply
/// looks like a valid date (starts with a 4-digit year), Err otherwise.
/// Matches TS's "starts with \\d{4}" sanity check and the INVALID
/// sentinel handling.
pub fn normalize_model_reply(raw: &str) -> DateTimeParseResult {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "INVALID" {
        return DateTimeParseResult::Err("Unable to parse date/time from input".into());
    }
    let bytes = trimmed.as_bytes();
    if bytes.len() < 4 || !bytes[..4].iter().all(u8::is_ascii_digit) {
        return DateTimeParseResult::Err("Unable to parse date/time from input".into());
    }
    DateTimeParseResult::Ok(trimmed.to_string())
}

fn weekday_name(w: chrono::Weekday) -> &'static str {
    match w {
        chrono::Weekday::Mon => "Monday",
        chrono::Weekday::Tue => "Tuesday",
        chrono::Weekday::Wed => "Wednesday",
        chrono::Weekday::Thu => "Thursday",
        chrono::Weekday::Fri => "Friday",
        chrono::Weekday::Sat => "Saturday",
        chrono::Weekday::Sun => "Sunday",
    }
}

fn format_timezone<Tz: chrono::TimeZone>(dt: &chrono::DateTime<Tz>) -> String {
    let offset_secs = dt.offset().fix().local_minus_utc();
    let sign = if offset_secs >= 0 { '+' } else { '-' };
    let abs = offset_secs.unsigned_abs() as u32;
    let hours = abs / 3600;
    let mins = (abs % 3600) / 60;
    format!("{}{:02}:{:02}", sign, hours, mins)
}

// Silence unused-use warnings from intermittent feature combinations.
#[allow(dead_code)]
fn _link_chrono_types(_: &chrono::DateTime<chrono::Utc>, _: chrono::Weekday) {
    // no-op: keeps `Datelike` / `Timelike` / `TimeZone` imports tied into
    // the compiled crate graph even if a chrono version drops one of them.
    let _ = Local.with_ymd_and_hms(2024, 1, 1, 0, 0, 0);
    let _ = chrono::Local::now().hour();
    let _ = chrono::Local::now().day();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_date_is_detected() {
        assert!(looks_like_iso_8601("2024-03-15"));
        assert!(looks_like_iso_8601("2024-03-15T14:30:00Z"));
        assert!(looks_like_iso_8601("  2024-03-15 "));
    }

    #[test]
    fn non_iso_rejected() {
        assert!(!looks_like_iso_8601("tomorrow"));
        assert!(!looks_like_iso_8601("03/15/2024"));
        assert!(!looks_like_iso_8601("2024"));
    }

    #[test]
    fn prompt_mentions_format() {
        let p = build_datetime_parser_prompt("tomorrow", DateTimeFormat::Date);
        assert!(p.contains("YYYY-MM-DD"));
        assert!(p.contains("User input: \"tomorrow\""));
    }

    #[test]
    fn prompt_includes_timezone_for_datetime() {
        let p = build_datetime_parser_prompt("next Monday", DateTimeFormat::DateTime);
        assert!(p.contains("YYYY-MM-DDTHH:MM:SS"));
    }

    #[test]
    fn system_prompt_verbatim_from_ts() {
        assert!(DATETIME_PARSER_SYSTEM_PROMPT.contains("ISO 8601"));
        assert!(DATETIME_PARSER_SYSTEM_PROMPT.contains("INVALID"));
    }

    #[test]
    fn normalize_rejects_empty_and_invalid() {
        assert!(matches!(normalize_model_reply(""), DateTimeParseResult::Err(_)));
        assert!(matches!(
            normalize_model_reply("INVALID"),
            DateTimeParseResult::Err(_)
        ));
        assert!(matches!(
            normalize_model_reply("whatever"),
            DateTimeParseResult::Err(_)
        ));
    }

    #[test]
    fn normalize_accepts_iso_date() {
        assert!(matches!(
            normalize_model_reply("2025-01-01"),
            DateTimeParseResult::Ok(_)
        ));
    }

    #[test]
    fn format_as_str_round_trip() {
        assert_eq!(DateTimeFormat::Date.as_str(), "date");
        assert_eq!(DateTimeFormat::DateTime.as_str(), "date-time");
    }
}
