use std::sync::OnceLock;
use ratatui::prelude::*;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

static SS: OnceLock<SyntaxSet> = OnceLock::new();
static TS: OnceLock<ThemeSet> = OnceLock::new();

fn ss() -> &'static SyntaxSet {
    SS.get_or_init(SyntaxSet::load_defaults_nonewlines)
}
fn ts() -> &'static ThemeSet {
    TS.get_or_init(ThemeSet::load_defaults)
}

/// Highlight a code line with syntect. Returns ratatui spans.
/// `ext` is the file extension ("rs", "ts", "py", etc.)
/// `bg` is an optional background override for diff lines.
pub fn highlight(code: &str, ext: Option<&str>, bg: Option<Color>) -> Vec<Span<'static>> {
    let ss = ss();
    let ts = ts();

    let syntax = ext
        .and_then(|e| ss.find_syntax_by_extension(e))
        .unwrap_or_else(|| ss.find_syntax_plain_text());

    // Use a theme that works well on dark terminals
    let theme_name = "base16-ocean.dark";
    let theme = ts.themes.get(theme_name)
        .or_else(|| ts.themes.values().next())
        .unwrap();

    let mut h = HighlightLines::new(syntax, theme);

    match h.highlight_line(code, ss) {
        Ok(ranges) if !ranges.is_empty() => ranges
            .into_iter()
            .filter(|(_, text)| !text.is_empty())
            .map(|(style, text)| {
                let fg = Color::Rgb(
                    style.foreground.r,
                    style.foreground.g,
                    style.foreground.b,
                );
                let mut s = Style::default().fg(fg);
                if let Some(b) = bg {
                    s = s.bg(b);
                }
                Span::styled(text.to_string(), s)
            })
            .collect(),
        _ => vec![Span::styled(
            code.to_string(),
            Style::default().bg(bg.unwrap_or(Color::Reset)),
        )],
    }
}
