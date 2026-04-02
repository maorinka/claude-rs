use ratatui::style::Color;
use std::fmt;
use std::str::FromStr;

/// All concrete theme names (no "auto").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeName {
    Dark,
    Light,
    DarkDaltonized,
    LightDaltonized,
    DarkAnsi,
    LightAnsi,
}

impl ThemeName {
    /// All variants in display order (matching TS THEME_NAMES).
    pub const ALL: &'static [ThemeName] = &[
        ThemeName::Dark,
        ThemeName::Light,
        ThemeName::DarkDaltonized,
        ThemeName::LightDaltonized,
        ThemeName::DarkAnsi,
        ThemeName::LightAnsi,
    ];
}

impl fmt::Display for ThemeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThemeName::Dark => write!(f, "dark"),
            ThemeName::Light => write!(f, "light"),
            ThemeName::DarkDaltonized => write!(f, "dark-daltonized"),
            ThemeName::LightDaltonized => write!(f, "light-daltonized"),
            ThemeName::DarkAnsi => write!(f, "dark-ansi"),
            ThemeName::LightAnsi => write!(f, "light-ansi"),
        }
    }
}

impl FromStr for ThemeName {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "dark" => Ok(ThemeName::Dark),
            "light" => Ok(ThemeName::Light),
            "dark-daltonized" => Ok(ThemeName::DarkDaltonized),
            "light-daltonized" => Ok(ThemeName::LightDaltonized),
            "dark-ansi" => Ok(ThemeName::DarkAnsi),
            "light-ansi" => Ok(ThemeName::LightAnsi),
            _ => Err(format!("unknown theme: {}", s)),
        }
    }
}

/// A theme preference as stored in user config. `Auto` follows the system
/// dark/light mode and is resolved to a ThemeName at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeSetting {
    Auto,
    Named(ThemeName),
}

impl ThemeSetting {
    /// All settings in picker display order.
    pub const ALL: &'static [ThemeSetting] = &[
        ThemeSetting::Auto,
        ThemeSetting::Named(ThemeName::Dark),
        ThemeSetting::Named(ThemeName::Light),
        ThemeSetting::Named(ThemeName::DarkDaltonized),
        ThemeSetting::Named(ThemeName::LightDaltonized),
        ThemeSetting::Named(ThemeName::DarkAnsi),
        ThemeSetting::Named(ThemeName::LightAnsi),
    ];

    /// Human-readable label for the picker.
    pub fn label(&self) -> &'static str {
        match self {
            ThemeSetting::Auto => "Auto (match terminal)",
            ThemeSetting::Named(ThemeName::Dark) => "Dark mode",
            ThemeSetting::Named(ThemeName::Light) => "Light mode",
            ThemeSetting::Named(ThemeName::DarkDaltonized) => "Dark mode (colorblind-friendly)",
            ThemeSetting::Named(ThemeName::LightDaltonized) => "Light mode (colorblind-friendly)",
            ThemeSetting::Named(ThemeName::DarkAnsi) => "Dark mode (ANSI colors only)",
            ThemeSetting::Named(ThemeName::LightAnsi) => "Light mode (ANSI colors only)",
        }
    }
}

impl fmt::Display for ThemeSetting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThemeSetting::Auto => write!(f, "auto"),
            ThemeSetting::Named(name) => write!(f, "{}", name),
        }
    }
}

impl FromStr for ThemeSetting {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "auto" => Ok(ThemeSetting::Auto),
            other => Ok(ThemeSetting::Named(ThemeName::from_str(other)?)),
        }
    }
}

/// Theme matching the original Claude Code terminal UI color palette.
/// Colors are specified as exact RGB values from the TypeScript source.
pub struct Theme {
    pub bg: Color,
    pub fg: Color,

    // Brand colors
    pub claude: Color,         // Claude orange -- spinner, branding
    pub claude_shimmer: Color, // Lighter orange for shimmer effect

    // Prompt
    pub prompt_border: Color, // Round border around input area

    // Semantic
    pub error: Color,
    pub warning: Color,
    pub success: Color,

    // Text hierarchy
    pub text: Color,         // Primary text
    pub inverse_text: Color, // Text on colored backgrounds
    pub inactive: Color,     // Dimmed secondary text
    pub subtle: Color,       // Very dim elements (dark gray in dark theme)
    pub muted: Color,        // General muted text (alias for inactive in rendering)

    // Message colors
    pub user_message_bg: Color, // Background for user message blocks
    pub tool_name: Color,       // Bold tool name color (none -- default text, bold)
    pub thinking: Color,        // Thinking text (dim)

    // Diff colors
    pub diff_added: Color,
    pub diff_removed: Color,

    // Layout
    pub border: Color, // General borders/separators

    // Permission
    pub permission: Color,
}

// ---------------------------------------------------------------------------
// Dark theme -- exact RGB from TS darkTheme
// ---------------------------------------------------------------------------

pub fn dark_theme() -> Theme {
    Theme {
        bg: Color::Reset,
        fg: Color::Reset,

        claude: Color::Rgb(215, 119, 87),
        claude_shimmer: Color::Rgb(235, 159, 127),

        prompt_border: Color::Rgb(136, 136, 136),

        error: Color::Rgb(255, 107, 128),
        warning: Color::Rgb(255, 193, 7),
        success: Color::Rgb(78, 186, 101),

        text: Color::Reset,
        inverse_text: Color::Rgb(0, 0, 0),
        inactive: Color::DarkGray,
        subtle: Color::Rgb(80, 80, 80),
        muted: Color::DarkGray,

        user_message_bg: Color::Rgb(55, 55, 55),
        tool_name: Color::Reset,
        thinking: Color::DarkGray,

        diff_added: Color::Rgb(34, 92, 43),
        diff_removed: Color::Rgb(122, 41, 54),

        border: Color::DarkGray,
        permission: Color::Rgb(177, 185, 249),
    }
}

// ---------------------------------------------------------------------------
// Light theme -- exact RGB from TS lightTheme
// ---------------------------------------------------------------------------

pub fn light_theme() -> Theme {
    Theme {
        bg: Color::Reset,
        fg: Color::Reset,

        claude: Color::Rgb(215, 119, 87),
        claude_shimmer: Color::Rgb(245, 149, 117),

        prompt_border: Color::Rgb(153, 153, 153),

        error: Color::Rgb(171, 43, 63),
        warning: Color::Rgb(150, 108, 30),
        success: Color::Rgb(44, 122, 57),

        text: Color::Reset,
        inverse_text: Color::Rgb(255, 255, 255),
        inactive: Color::Gray,
        subtle: Color::Rgb(175, 175, 175),
        muted: Color::Gray,

        user_message_bg: Color::Rgb(240, 240, 240),
        tool_name: Color::Reset,
        thinking: Color::Gray,

        diff_added: Color::Rgb(105, 219, 124),
        diff_removed: Color::Rgb(255, 168, 180),

        border: Color::Gray,
        permission: Color::Rgb(87, 105, 247),
    }
}

// ---------------------------------------------------------------------------
// Dark daltonized (color-blind friendly) -- exact RGB from TS
// ---------------------------------------------------------------------------

pub fn dark_daltonized_theme() -> Theme {
    Theme {
        bg: Color::Reset,
        fg: Color::Reset,

        claude: Color::Rgb(255, 153, 51),       // Orange adjusted for deuteranopia
        claude_shimmer: Color::Rgb(255, 183, 101),

        prompt_border: Color::Rgb(136, 136, 136),

        error: Color::Rgb(255, 102, 102),        // Bright red
        warning: Color::Rgb(255, 204, 0),        // Yellow-orange for deuteranopia
        success: Color::Rgb(51, 153, 255),       // Blue instead of green

        text: Color::Reset,
        inverse_text: Color::Rgb(0, 0, 0),
        inactive: Color::Rgb(153, 153, 153),
        subtle: Color::Rgb(80, 80, 80),
        muted: Color::Rgb(153, 153, 153),

        user_message_bg: Color::Rgb(55, 55, 55),
        tool_name: Color::Reset,
        thinking: Color::Rgb(153, 153, 153),

        diff_added: Color::Rgb(0, 68, 102),      // Dark blue
        diff_removed: Color::Rgb(102, 0, 0),     // Dark red

        border: Color::Rgb(80, 80, 80),
        permission: Color::Rgb(153, 204, 255),   // Light blue
    }
}

// ---------------------------------------------------------------------------
// Light daltonized (color-blind friendly) -- exact RGB from TS
// ---------------------------------------------------------------------------

pub fn light_daltonized_theme() -> Theme {
    Theme {
        bg: Color::Reset,
        fg: Color::Reset,

        claude: Color::Rgb(255, 153, 51),        // Orange adjusted for deuteranopia
        claude_shimmer: Color::Rgb(255, 183, 101),

        prompt_border: Color::Rgb(153, 153, 153),

        error: Color::Rgb(204, 0, 0),            // Pure red for better distinction
        warning: Color::Rgb(255, 153, 0),        // Orange adjusted for deuteranopia
        success: Color::Rgb(0, 102, 153),        // Blue instead of green

        text: Color::Reset,
        inverse_text: Color::Rgb(255, 255, 255),
        inactive: Color::Rgb(102, 102, 102),
        subtle: Color::Rgb(175, 175, 175),
        muted: Color::Rgb(102, 102, 102),

        user_message_bg: Color::Rgb(220, 220, 220),
        tool_name: Color::Reset,
        thinking: Color::Rgb(102, 102, 102),

        diff_added: Color::Rgb(153, 204, 255),   // Light blue instead of green
        diff_removed: Color::Rgb(255, 204, 204), // Light red

        border: Color::Rgb(175, 175, 175),
        permission: Color::Rgb(51, 102, 255),    // Bright blue
    }
}

// ---------------------------------------------------------------------------
// Dark ANSI -- uses only the 16 standard ANSI colors (from TS darkAnsiTheme)
// ---------------------------------------------------------------------------

pub fn dark_ansi_theme() -> Theme {
    Theme {
        bg: Color::Reset,
        fg: Color::Reset,

        claude: Color::LightRed,                  // ansi:redBright
        claude_shimmer: Color::LightYellow,        // ansi:yellowBright

        prompt_border: Color::White,               // ansi:white

        error: Color::LightRed,                    // ansi:redBright
        warning: Color::LightYellow,               // ansi:yellowBright
        success: Color::LightGreen,                // ansi:greenBright

        text: Color::Reset,                        // ansi:whiteBright (terminal default on dark)
        inverse_text: Color::Black,                // ansi:black
        inactive: Color::White,                    // ansi:white
        subtle: Color::White,                      // ansi:white
        muted: Color::White,

        user_message_bg: Color::DarkGray,          // ansi:blackBright
        tool_name: Color::Reset,
        thinking: Color::White,                    // ansi:white

        diff_added: Color::Green,                  // ansi:green
        diff_removed: Color::Red,                  // ansi:red

        border: Color::White,                      // ansi:white
        permission: Color::LightBlue,              // ansi:blueBright
    }
}

// ---------------------------------------------------------------------------
// Light ANSI -- uses only the 16 standard ANSI colors (from TS lightAnsiTheme)
// ---------------------------------------------------------------------------

pub fn light_ansi_theme() -> Theme {
    Theme {
        bg: Color::Reset,
        fg: Color::Reset,

        claude: Color::LightRed,                   // ansi:redBright
        claude_shimmer: Color::LightYellow,         // ansi:yellowBright

        prompt_border: Color::White,                // ansi:white

        error: Color::Red,                          // ansi:red
        warning: Color::Yellow,                     // ansi:yellow
        success: Color::Green,                      // ansi:green

        text: Color::Reset,                         // ansi:black (terminal default on light)
        inverse_text: Color::White,                 // ansi:white
        inactive: Color::DarkGray,                  // ansi:blackBright
        subtle: Color::DarkGray,                    // ansi:blackBright
        muted: Color::DarkGray,

        user_message_bg: Color::White,              // ansi:white
        tool_name: Color::Reset,
        thinking: Color::DarkGray,                  // ansi:blackBright

        diff_added: Color::Green,                   // ansi:green
        diff_removed: Color::Red,                   // ansi:red

        border: Color::DarkGray,                    // ansi:blackBright
        permission: Color::Blue,                    // ansi:blue
    }
}

// ---------------------------------------------------------------------------
// Theme resolution
// ---------------------------------------------------------------------------

/// Get the theme for a concrete ThemeName.
pub fn get_theme(name: ThemeName) -> Theme {
    match name {
        ThemeName::Dark => dark_theme(),
        ThemeName::Light => light_theme(),
        ThemeName::DarkDaltonized => dark_daltonized_theme(),
        ThemeName::LightDaltonized => light_daltonized_theme(),
        ThemeName::DarkAnsi => dark_ansi_theme(),
        ThemeName::LightAnsi => light_ansi_theme(),
    }
}

/// Resolve a ThemeSetting (which may be Auto) to a concrete Theme.
pub fn resolve_theme(setting: ThemeSetting) -> Theme {
    match setting {
        ThemeSetting::Auto => get_theme(detect_system_theme()),
        ThemeSetting::Named(name) => get_theme(name),
    }
}

/// Detect whether the system/terminal prefers dark or light mode.
///
/// Strategy (matching TS systemTheme.ts):
/// 1. COLORFGBG env var (rxvt convention: bg 0-6 or 8 = dark, 7 or 9-15 = light)
/// 2. macOS: `defaults read -g AppleInterfaceStyle`
/// 3. Default to dark (most developer terminals are dark)
pub fn detect_system_theme() -> ThemeName {
    // 1. COLORFGBG
    if let Ok(val) = std::env::var("COLORFGBG") {
        if let Some(bg) = val.rsplit(';').next().and_then(|s| s.parse::<u32>().ok()) {
            if bg <= 6 || bg == 8 {
                return ThemeName::Dark;
            } else {
                return ThemeName::Light;
            }
        }
    }

    // 2. macOS dark mode detection
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.trim().eq_ignore_ascii_case("dark") {
                return ThemeName::Dark;
            }
            if output.status.success() {
                return ThemeName::Light;
            }
            // Command failed => key doesn't exist => macOS light mode,
            // but most terminals use dark backgrounds anyway.
        }
    }

    // 3. Default to dark
    ThemeName::Dark
}

/// Legacy detection function for backward compatibility.
pub fn detect_theme() -> Theme {
    resolve_theme(ThemeSetting::Auto)
}

/// Return sample colors for a theme setting (used by the theme picker preview).
/// Returns (text_color, accent_color, success_color, error_color, border_color).
pub fn preview_colors(setting: ThemeSetting) -> (Color, Color, Color, Color, Color) {
    let t = resolve_theme(setting);
    (t.claude, t.permission, t.success, t.error, t.prompt_border)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_claude_orange() {
        let t = dark_theme();
        assert_eq!(t.claude, Color::Rgb(215, 119, 87));
    }

    #[test]
    fn dark_theme_error_color() {
        let t = dark_theme();
        assert_eq!(t.error, Color::Rgb(255, 107, 128));
    }

    #[test]
    fn dark_theme_success_color() {
        let t = dark_theme();
        assert_eq!(t.success, Color::Rgb(78, 186, 101));
    }

    #[test]
    fn dark_theme_user_message_bg() {
        let t = dark_theme();
        assert_eq!(t.user_message_bg, Color::Rgb(55, 55, 55));
    }

    #[test]
    fn light_theme_permission() {
        let t = light_theme();
        assert_eq!(t.permission, Color::Rgb(87, 105, 247));
    }

    #[test]
    fn all_theme_settings_count() {
        assert_eq!(ThemeSetting::ALL.len(), 7); // auto + 6 themes
    }

    #[test]
    fn theme_setting_round_trip() {
        for &setting in ThemeSetting::ALL {
            let s = setting.to_string();
            let parsed: ThemeSetting = s.parse().unwrap();
            assert_eq!(parsed, setting);
        }
    }

    #[test]
    fn daltonized_uses_blue_for_success() {
        // Colorblind-friendly themes use blue instead of green for success
        let dt = dark_daltonized_theme();
        assert_eq!(dt.success, Color::Rgb(51, 153, 255));
        let lt = light_daltonized_theme();
        assert_eq!(lt.success, Color::Rgb(0, 102, 153));
    }

    #[test]
    fn ansi_themes_use_named_colors() {
        let da = dark_ansi_theme();
        assert_eq!(da.error, Color::LightRed);
        let la = light_ansi_theme();
        assert_eq!(la.error, Color::Red);
    }
}
