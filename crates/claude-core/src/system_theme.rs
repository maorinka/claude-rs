//! Terminal dark/light detection for the `auto` theme setting.
//!
//! Port of TS `utils/systemTheme.ts:1-119`.
//!
//! Detection is based on the terminal's **background colour**
//! (queried via OSC 11 elsewhere), NOT the OS appearance setting —
//! a dark terminal on a light-mode OS should still resolve to
//! `dark`. This module holds:
//! - A process-wide cached `SystemTheme`, seeded from
//!   `$COLORFGBG` synchronously and updated by the watcher once the
//!   OSC 11 round-trip returns.
//! - [`theme_from_osc_color`]: parse the colour string returned by
//!   the terminal and classify it via BT.709 relative luminance.

use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemTheme {
    Dark,
    Light,
}

static CACHED: Mutex<Option<SystemTheme>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<SystemTheme>> {
    CACHED.lock().unwrap_or_else(|p| p.into_inner())
}

/// Get the cached theme, seeding from `$COLORFGBG` on first call.
/// TS `getSystemThemeName()`. Defaults to `Dark` when nothing is
/// resolvable (matches TS default at `systemTheme.ts:26`).
pub fn get_system_theme_name() -> SystemTheme {
    let mut guard = lock();
    if let Some(t) = *guard {
        return t;
    }
    let t = detect_from_colorfgbg().unwrap_or(SystemTheme::Dark);
    *guard = Some(t);
    t
}

/// Called by the watcher once the OSC 11 response arrives so non-UI
/// call sites stay in sync. TS `setCachedSystemTheme`.
pub fn set_cached_system_theme(theme: SystemTheme) {
    *lock() = Some(theme);
}

/// Test-only helper — the OnceLock-style cache needs a way to clear
/// between cases.
#[cfg(test)]
fn clear_cache() {
    *lock() = None;
}

/// Parse an OSC colour response into a theme classification.
///
/// Accepts:
/// - `rgb:RRRR/GGGG/BBBB` (xterm / iTerm2 / Terminal.app / Ghostty /
///   kitty / Alacritty …). Each component is 1–4 hex digits scaled
///   to `[0, 16^n - 1]`. `rgba:…/…/…/…` alpha is parsed-but-ignored.
/// - `#RRGGBB` / `#RRRRGGGGBBBB` — rare but cheap to accept.
///
/// Returns `None` for unrecognised formats so the caller can fall
/// back to `$COLORFGBG` / the default.
pub fn theme_from_osc_color(data: &str) -> Option<SystemTheme> {
    let rgb = parse_osc_rgb(data)?;
    // ITU-R BT.709 relative luminance. Midpoint split: > 0.5 is light.
    let luminance = 0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b;
    Some(if luminance > 0.5 {
        SystemTheme::Light
    } else {
        SystemTheme::Dark
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rgb {
    r: f64,
    g: f64,
    b: f64,
}

fn parse_osc_rgb(data: &str) -> Option<Rgb> {
    if let Some(rgb) = parse_rgb_colon(data) {
        return Some(rgb);
    }
    parse_hash_hex(data)
}

fn parse_rgb_colon(data: &str) -> Option<Rgb> {
    let re =
        regex::Regex::new(r"(?i)^rgba?:([0-9a-f]{1,4})/([0-9a-f]{1,4})/([0-9a-f]{1,4})").ok()?;
    let cap = re.captures(data)?;
    Some(Rgb {
        r: hex_component(cap.get(1)?.as_str()),
        g: hex_component(cap.get(2)?.as_str()),
        b: hex_component(cap.get(3)?.as_str()),
    })
}

fn parse_hash_hex(data: &str) -> Option<Rgb> {
    // Split a `#RRGGBB`-style string into three equal runs.
    let body = data.strip_prefix('#')?;
    if body.is_empty() || body.len() % 3 != 0 {
        return None;
    }
    if !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let n = body.len() / 3;
    let r = &body[..n];
    let g = &body[n..2 * n];
    let b = &body[2 * n..];
    Some(Rgb {
        r: hex_component(r),
        g: hex_component(g),
        b: hex_component(b),
    })
}

fn hex_component(hex: &str) -> f64 {
    let n = u32::from_str_radix(hex, 16).unwrap_or(0) as f64;
    // 16^len - 1 — the per-component max for `len` hex digits.
    let max = 16u32.pow(hex.len() as u32) as f64 - 1.0;
    if max == 0.0 {
        0.0
    } else {
        n / max
    }
}

fn detect_from_colorfgbg() -> Option<SystemTheme> {
    let raw = std::env::var("COLORFGBG").ok()?;
    let bg = raw.split(';').next_back()?;
    if bg.is_empty() {
        return None;
    }
    let n: i32 = bg.parse().ok()?;
    if !(0..=15).contains(&n) {
        return None;
    }
    // rxvt convention: ANSI 0–6 + 8 are dark; 7 (white) + 9–15 (bright) are
    // light. Matches TS `systemTheme.ts:118`.
    Some(if n <= 6 || n == 8 {
        SystemTheme::Dark
    } else {
        SystemTheme::Light
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn rgb_black_is_dark() {
        assert_eq!(
            theme_from_osc_color("rgb:0000/0000/0000"),
            Some(SystemTheme::Dark)
        );
        assert_eq!(
            theme_from_osc_color("rgb:00/00/00"),
            Some(SystemTheme::Dark)
        );
    }

    #[test]
    fn rgb_white_is_light() {
        assert_eq!(
            theme_from_osc_color("rgb:ffff/ffff/ffff"),
            Some(SystemTheme::Light)
        );
    }

    #[test]
    fn rgb_mid_grey_on_light_side() {
        // 0x8080 / 0xffff ≈ 0.502 — just over the midpoint, so light.
        assert_eq!(
            theme_from_osc_color("rgb:8080/8080/8080"),
            Some(SystemTheme::Light)
        );
    }

    #[test]
    fn hash_hex_rrggbb() {
        assert_eq!(theme_from_osc_color("#000000"), Some(SystemTheme::Dark));
        assert_eq!(theme_from_osc_color("#ffffff"), Some(SystemTheme::Light));
    }

    #[test]
    fn hash_hex_rrrrggggbbbb() {
        assert_eq!(
            theme_from_osc_color("#ffffffffffff"),
            Some(SystemTheme::Light)
        );
    }

    #[test]
    fn rgba_alpha_ignored() {
        // rgba:red/green/blue/alpha — the trailing component is parsed-
        // but-ignored, so classification depends only on RGB.
        assert_eq!(
            theme_from_osc_color("rgba:ffff/ffff/ffff/0000"),
            Some(SystemTheme::Light)
        );
    }

    #[test]
    fn unrecognised_format_returns_none() {
        assert_eq!(theme_from_osc_color(""), None);
        assert_eq!(theme_from_osc_color("hello"), None);
        assert_eq!(theme_from_osc_color("#ghi"), None);
        // Length not a multiple of 3:
        assert_eq!(theme_from_osc_color("#ff"), None);
    }

    #[test]
    fn colorfgbg_dark_bg() {
        let _g = lock_env();
        clear_cache();
        std::env::set_var("COLORFGBG", "15;0");
        assert_eq!(get_system_theme_name(), SystemTheme::Dark);
        std::env::remove_var("COLORFGBG");
        clear_cache();
    }

    #[test]
    fn colorfgbg_light_bg() {
        let _g = lock_env();
        clear_cache();
        std::env::set_var("COLORFGBG", "0;15");
        assert_eq!(get_system_theme_name(), SystemTheme::Light);
        std::env::remove_var("COLORFGBG");
        clear_cache();
    }

    #[test]
    fn colorfgbg_three_segment_uses_last() {
        // Some terminals emit `fg;default;bg`.
        let _g = lock_env();
        clear_cache();
        std::env::set_var("COLORFGBG", "0;default;15");
        assert_eq!(get_system_theme_name(), SystemTheme::Light);
        std::env::remove_var("COLORFGBG");
        clear_cache();
    }

    #[test]
    fn colorfgbg_unset_defaults_to_dark() {
        let _g = lock_env();
        clear_cache();
        std::env::remove_var("COLORFGBG");
        assert_eq!(get_system_theme_name(), SystemTheme::Dark);
        clear_cache();
    }

    #[test]
    fn colorfgbg_out_of_range_rejected() {
        let _g = lock_env();
        clear_cache();
        std::env::set_var("COLORFGBG", "0;99");
        // Bad bg index → defaults (dark).
        assert_eq!(get_system_theme_name(), SystemTheme::Dark);
        std::env::remove_var("COLORFGBG");
        clear_cache();
    }

    #[test]
    fn set_cached_overrides_detection() {
        let _g = lock_env();
        clear_cache();
        set_cached_system_theme(SystemTheme::Light);
        assert_eq!(get_system_theme_name(), SystemTheme::Light);
        set_cached_system_theme(SystemTheme::Dark);
        assert_eq!(get_system_theme_name(), SystemTheme::Dark);
        clear_cache();
    }
}
