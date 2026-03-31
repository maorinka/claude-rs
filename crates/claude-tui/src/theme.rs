use ratatui::style::Color;

pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub accent: Color,
    pub error: Color,
    pub warning: Color,
    pub success: Color,
    pub muted: Color,
    pub border: Color,
    pub user_message: Color,
    pub assistant_message: Color,
    pub tool_name: Color,
    pub thinking: Color,
}

pub fn dark_theme() -> Theme {
    Theme {
        bg: Color::Reset,
        fg: Color::White,
        accent: Color::Cyan,
        error: Color::Red,
        warning: Color::Yellow,
        success: Color::Green,
        muted: Color::DarkGray,
        border: Color::DarkGray,
        user_message: Color::Blue,
        assistant_message: Color::White,
        tool_name: Color::Magenta,
        thinking: Color::DarkGray,
    }
}

pub fn light_theme() -> Theme {
    Theme {
        bg: Color::Reset,
        fg: Color::Black,
        accent: Color::Blue,
        error: Color::Red,
        warning: Color::Yellow,
        success: Color::Green,
        muted: Color::Gray,
        border: Color::Gray,
        user_message: Color::Blue,
        assistant_message: Color::Black,
        tool_name: Color::Magenta,
        thinking: Color::Gray,
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
