//! Memory-age math + freshness text. Port of `src/memdir/memoryAge.ts`.
//!
//! Models are poor at date arithmetic — a raw ISO timestamp doesn't trigger
//! staleness reasoning the way "47 days ago" does. This module produces
//! human-readable strings and a staleness caveat for memories >1 day old.

const MS_PER_DAY: u64 = 86_400_000;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Days elapsed since `mtime_ms`. Floor-rounded — 0 for today, 1 for
/// yesterday, 2+ for older. Negative input (future mtime / clock skew)
/// clamps to 0.
pub fn memory_age_days(mtime_ms: u64) -> u64 {
    let now = now_ms();
    if now <= mtime_ms {
        return 0;
    }
    (now - mtime_ms) / MS_PER_DAY
}

/// Human-readable age string: "today" / "yesterday" / "N days ago".
pub fn memory_age(mtime_ms: u64) -> String {
    match memory_age_days(mtime_ms) {
        0 => "today".into(),
        1 => "yesterday".into(),
        d => format!("{} days ago", d),
    }
}

/// Plain-text staleness caveat for memories >1 day old. Returns empty
/// string for fresh (today/yesterday) memories — warning there is noise.
pub fn memory_freshness_text(mtime_ms: u64) -> String {
    let d = memory_age_days(mtime_ms);
    if d <= 1 {
        return String::new();
    }
    format!(
        "This memory is {d} days old. \
         Memories are point-in-time observations, not live state — \
         claims about code behavior or file:line citations may be outdated. \
         Verify against current code before asserting as fact."
    )
}

/// Per-memory staleness note wrapped in <system-reminder> tags. Returns
/// empty string for memories ≤ 1 day old.
pub fn memory_freshness_note(mtime_ms: u64) -> String {
    let text = memory_freshness_text(mtime_ms);
    if text.is_empty() {
        return String::new();
    }
    format!("<system-reminder>{text}</system-reminder>\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n_days_ago(n: u64) -> u64 {
        now_ms().saturating_sub(n * MS_PER_DAY)
    }

    #[test]
    fn today_is_zero_days() {
        assert_eq!(memory_age_days(now_ms()), 0);
        assert_eq!(memory_age(now_ms()), "today");
    }

    #[test]
    fn yesterday_is_one_day() {
        let ts = n_days_ago(1) + 1_000; // 1 day ago + a bit
        assert!(memory_age_days(ts) <= 1);
    }

    #[test]
    fn freshness_empty_when_fresh() {
        assert_eq!(memory_freshness_text(now_ms()), "");
        assert_eq!(memory_freshness_note(now_ms()), "");
    }

    #[test]
    fn freshness_wraps_when_stale() {
        let old = n_days_ago(10);
        assert!(memory_freshness_text(old).contains("10 days old"));
        assert!(memory_freshness_note(old).starts_with("<system-reminder>"));
    }

    #[test]
    fn future_mtime_clamps_to_zero() {
        let future = now_ms() + 10 * MS_PER_DAY;
        assert_eq!(memory_age_days(future), 0);
    }
}
