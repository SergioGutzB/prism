use ratatui::style::Color;

/// A complete color palette for a theme.
#[derive(Debug, Clone)]
pub struct Theme {
    pub background: Color,
    pub foreground: Color,
    pub selected_bg: Color,
    pub selected_fg: Color,
    pub diff_add: Color,
    pub diff_remove: Color,
    pub diff_context: Color,
    pub diff_hunk: Color,
    pub critical: Color,
    pub warning: Color,
    pub suggestion: Color,
    pub praise: Color,
    pub agent_running: Color,
    pub agent_done: Color,
    pub agent_failed: Color,
    pub agent_skipped: Color,
    pub agent_disabled: Color,
    pub border: Color,
    pub border_focused: Color,
    pub title: Color,
    pub label_bg: Color,
    pub label_fg: Color,
    pub status_bar_bg: Color,
    pub status_bar_fg: Color,
    pub keybind_key: Color,
    pub keybind_desc: Color,
    pub loading: Color,
    pub muted: Color,
    pub highlight: Color,
}

impl Theme {
    /// Return the dark theme (default).
    pub fn dark() -> Self {
        Self {
            background: Color::Black,
            foreground: Color::White,
            selected_bg: Color::Blue,
            selected_fg: Color::White,
            diff_add: Color::Green,
            diff_remove: Color::Red,
            diff_context: Color::DarkGray,
            diff_hunk: Color::Cyan,
            critical: Color::Red,
            warning: Color::Yellow,
            suggestion: Color::Cyan,
            praise: Color::Green,
            agent_running: Color::Yellow,
            agent_done: Color::Green,
            agent_failed: Color::Red,
            agent_skipped: Color::DarkGray,
            agent_disabled: Color::DarkGray,
            border: Color::DarkGray,
            border_focused: Color::Blue,
            title: Color::Cyan,
            label_bg: Color::DarkGray,
            label_fg: Color::White,
            status_bar_bg: Color::DarkGray,
            status_bar_fg: Color::White,
            keybind_key: Color::Cyan,
            keybind_desc: Color::DarkGray,
            loading: Color::Yellow,
            muted: Color::DarkGray,
            highlight: Color::Yellow,
        }
    }

    /// Return the light theme.
    pub fn light() -> Self {
        Self {
            background: Color::White,
            foreground: Color::Black,
            selected_bg: Color::LightBlue,
            selected_fg: Color::Black,
            diff_add: Color::Green,
            diff_remove: Color::Red,
            diff_context: Color::Gray,
            diff_hunk: Color::Blue,
            critical: Color::Red,
            warning: Color::Yellow,
            suggestion: Color::Blue,
            praise: Color::Green,
            agent_running: Color::Yellow,
            agent_done: Color::Green,
            agent_failed: Color::Red,
            agent_skipped: Color::Gray,
            agent_disabled: Color::Gray,
            border: Color::Gray,
            border_focused: Color::Blue,
            title: Color::Blue,
            label_bg: Color::LightBlue,
            label_fg: Color::Black,
            status_bar_bg: Color::Gray,
            status_bar_fg: Color::Black,
            keybind_key: Color::Blue,
            keybind_desc: Color::Gray,
            loading: Color::Yellow,
            muted: Color::Gray,
            highlight: Color::Yellow,
        }
    }

    /// Pick theme by name from config.
    pub fn current(name: &str) -> Self {
        match name {
            "light" => Self::light(),
            _ => Self::dark(),
        }
    }
}
