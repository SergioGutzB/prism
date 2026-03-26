use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::ui::theme::Theme;

/// Render a horizontal keybind bar at the bottom of the screen.
///
/// `bindings` is a slice of `(key, description)` pairs.
pub fn render(frame: &mut Frame, area: Rect, bindings: &[(&str, &str)], t: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border))
        .style(Style::default().bg(t.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut spans: Vec<Span> = Vec::new();
    for (i, (key, desc)) in bindings.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", Style::default()));
        }
        spans.push(Span::styled(
            *key,
            Style::default()
                .fg(t.keybind_key)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" ", Style::default()));
        spans.push(Span::styled(*desc, Style::default().fg(t.keybind_desc)));
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), inner);
}
