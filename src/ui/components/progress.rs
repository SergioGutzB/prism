use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Gauge};

use crate::ui::theme::Theme;

/// Render a labeled progress bar.
pub fn render(frame: &mut Frame, area: Rect, t: &Theme, ratio: f64, label: &str, title: &str) {
    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(title)
                .title_style(Style::default().fg(t.title))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.border))
                .style(Style::default().bg(t.background)),
        )
        .gauge_style(Style::default().fg(t.agent_done).bg(t.background))
        .label(label)
        .ratio(ratio.clamp(0.0, 1.0));

    frame.render_widget(gauge, area);
}
