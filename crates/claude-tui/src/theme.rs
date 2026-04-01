use ratatui::style::Color;

/// Theme matching the original Claude Code terminal UI color palette.
/// Colors are specified as exact RGB values from the TypeScript source.
pub struct Theme {
    pub bg: Color,
    pub fg: Color,

    // Brand colors
    pub claude: Color,         // Claude orange — spinner, branding
    pub claude_shimmer: Color, // Lighter orange for shimmer effect

    // Prompt
    pub prompt_border: Color,  // Round border around input area

    // Semantic
    pub error: Color,
    pub warning: Color,
    pub success: Color,

    // Text hierarchy
    pub text: Color,           // Primary text
    pub inverse_text: Color,   // Text on colored backgrounds
    pub inactive: Color,       // Dimmed secondary text
    pub subtle: Color,         // Very dim elements (dark gray in dark theme)
    pub muted: Color,          // General muted text (alias for inactive in rendering)

    // Message colors
    pub user_message_bg: Color,    // Background for user message blocks
    pub tool_name: Color,          // Bold tool name color (none — default text, bold)
    pub thinking: Color,           // Thinking text (dim)

    // Diff colors
    pub diff_added: Color,
    pub diff_removed: Color,

    // Layout
    pub border: Color,         // General borders/separators

    // Permission
    pub permission: Color,
}

/// Dark theme using exact RGB values from the original Claude Code dark theme.
pub fn dark_theme() -> Theme {
    Theme {
        bg: Color::Reset,
        fg: Color::Rgb(255, 255, 255),

        claude: Color::Rgb(215, 119, 87),        // Claude orange
        claude_shimmer: Color::Rgb(235, 159, 127),

        prompt_border: Color::Rgb(136, 136, 136), // Medium gray

        error: Color::Rgb(255, 107, 128),         // Bright red
        warning: Color::Rgb(255, 193, 7),          // Bright amber
        success: Color::Rgb(78, 186, 101),         // Bright green

        text: Color::Rgb(255, 255, 255),           // White
        inverse_text: Color::Rgb(0, 0, 0),         // Black
        inactive: Color::Rgb(153, 153, 153),       // Light gray
        subtle: Color::Rgb(80, 80, 80),            // Dark gray
        muted: Color::Rgb(153, 153, 153),          // Same as inactive

        user_message_bg: Color::Rgb(55, 55, 55),  // Lighter grey for user msgs
        tool_name: Color::Rgb(255, 255, 255),      // Tool names are bold white (default text)
        thinking: Color::Rgb(153, 153, 153),       // Dim/inactive

        diff_added: Color::Rgb(34, 92, 43),
        diff_removed: Color::Rgb(122, 41, 54),

        border: Color::Rgb(136, 136, 136),         // Same as prompt_border
        permission: Color::Rgb(177, 185, 249),     // Light blue-purple
    }
}

/// Light theme using exact RGB values from the original Claude Code light theme.
pub fn light_theme() -> Theme {
    Theme {
        bg: Color::Reset,
        fg: Color::Rgb(0, 0, 0),

        claude: Color::Rgb(215, 119, 87),          // Claude orange
        claude_shimmer: Color::Rgb(245, 149, 117),

        prompt_border: Color::Rgb(153, 153, 153),  // Medium gray

        error: Color::Rgb(171, 43, 63),            // Red
        warning: Color::Rgb(150, 108, 30),         // Amber
        success: Color::Rgb(44, 122, 57),          // Green

        text: Color::Rgb(0, 0, 0),                 // Black
        inverse_text: Color::Rgb(255, 255, 255),   // White
        inactive: Color::Rgb(102, 102, 102),       // Dark gray
        subtle: Color::Rgb(175, 175, 175),         // Light gray
        muted: Color::Rgb(102, 102, 102),          // Same as inactive

        user_message_bg: Color::Rgb(240, 240, 240),
        tool_name: Color::Rgb(0, 0, 0),            // Bold black
        thinking: Color::Rgb(102, 102, 102),

        diff_added: Color::Rgb(105, 219, 124),
        diff_removed: Color::Rgb(255, 168, 180),

        border: Color::Rgb(153, 153, 153),
        permission: Color::Rgb(87, 105, 247),
    }
}

pub fn detect_theme() -> Theme {
    // Check COLORFGBG env var, default to dark
    if let Ok(val) = std::env::var("COLORFGBG") {
        if val.ends_with(";15") || val.ends_with(";7") {
            return light_theme();
        }
    }
    dark_theme()
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
    fn light_theme_text_black() {
        let t = light_theme();
        assert_eq!(t.text, Color::Rgb(0, 0, 0));
    }

    #[test]
    fn detect_defaults_to_dark() {
        // Without COLORFGBG set in test env, should default to dark
        let t = detect_theme();
        // Dark theme has white text
        assert_eq!(t.text, Color::Rgb(255, 255, 255));
    }
}
