//! Port of `src/constants/common.ts`.
//!
//! Local ISO date helpers honouring the `CLAUDE_CODE_OVERRIDE_DATE` env
//! var (ant-only). Uses `chrono::Local` for the timezone-aware formatting
//! the TS `toLocaleString` call gives you for free.

use chrono::{Datelike, Local, NaiveDate};

/// Return today's local date as `YYYY-MM-DD`. Respects
/// `CLAUDE_CODE_OVERRIDE_DATE` so tests / ant-internal tools can pin it.
pub fn get_local_iso_date() -> String {
    if let Ok(override_date) = std::env::var("CLAUDE_CODE_OVERRIDE_DATE") {
        if !override_date.is_empty() {
            return override_date;
        }
    }
    let now = Local::now();
    format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day())
}

/// Return `"<month-name> <YYYY>"` in the local timezone, e.g.
/// "February 2026". Changes monthly so the prompt-cache key isn't blown
/// on a daily cadence.
pub fn get_local_month_year() -> String {
    let (year, month) = if let Ok(override_date) = std::env::var("CLAUDE_CODE_OVERRIDE_DATE") {
        if let Ok(d) = NaiveDate::parse_from_str(&override_date, "%Y-%m-%d") {
            (d.year(), d.month())
        } else {
            let now = Local::now();
            (now.year(), now.month())
        }
    } else {
        let now = Local::now();
        (now.year(), now.month())
    };

    let name = match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "Unknown",
    };
    format!("{} {}", name, year)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_env_var_is_honoured() {
        std::env::set_var("CLAUDE_CODE_OVERRIDE_DATE", "2026-04-18");
        assert_eq!(get_local_iso_date(), "2026-04-18");
        assert_eq!(get_local_month_year(), "April 2026");
        std::env::remove_var("CLAUDE_CODE_OVERRIDE_DATE");
    }

    #[test]
    fn iso_format_shape() {
        std::env::remove_var("CLAUDE_CODE_OVERRIDE_DATE");
        let s = get_local_iso_date();
        assert_eq!(s.len(), 10);
        assert!(s.chars().nth(4) == Some('-'));
        assert!(s.chars().nth(7) == Some('-'));
    }
}
