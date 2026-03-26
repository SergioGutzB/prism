use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::App;
use crate::ui::theme::Theme;

/// Parse a unified diff string into colored lines.
pub fn parse_diff_lines(diff: &str, t: &Theme) -> Vec<Line<'static>> {
    diff.lines()
        .map(|raw_line| {
            let line = raw_line.to_string();
            if line.starts_with("@@") {
                Line::from(Span::styled(line, Style::default().fg(t.diff_hunk)))
            } else if line.starts_with('+') {
                Line::from(Span::styled(line, Style::default().fg(t.diff_add)))
            } else if line.starts_with('-') {
                Line::from(Span::styled(line, Style::default().fg(t.diff_remove)))
            } else if line.starts_with("diff ") || line.starts_with("index ") || line.starts_with("---") || line.starts_with("+++") {
                Line::from(Span::styled(line, Style::default().fg(t.title)))
            } else {
                Line::from(Span::styled(line, Style::default().fg(t.diff_context)))
            }
        })
        .collect()
}

/// Render the diff view panel.
pub fn render(frame: &mut Frame, app: &App, area: Rect, t: &Theme, focused: bool) {
    let border_color = if focused { t.border_focused } else { t.border };

    let block = Block::default()
        .title(" Diff ")
        .title_style(Style::default().fg(t.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.background));

    match &app.current_diff {
        None => {
            if app.pr_loading {
                let inner = block.inner(area);
                frame.render_widget(block, area);
                frame.render_widget(
                    Paragraph::new(format!("{} Loading diff…", app.spinner_char()))
                        .style(Style::default().fg(t.loading)),
                    inner,
                );
            } else {
                let inner = block.inner(area);
                frame.render_widget(block, area);
                frame.render_widget(
                    Paragraph::new("No diff available.")
                        .style(Style::default().fg(t.muted)),
                    inner,
                );
            }
        }
        Some(diff) => {
            let lines = parse_diff_lines(diff, t);
            let total_lines = lines.len() as u16;
            let scroll = app.diff_scroll.min(total_lines.saturating_sub(1));

            let para = Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0));

            frame.render_widget(para, area);

            // Scrollbar hint
            if total_lines > 0 {
                let scroll_pct = scroll * 100 / total_lines.max(1);
                let hint = format!(" {scroll_pct}% ");
                let hint_area = Rect {
                    x: area.right().saturating_sub(hint.len() as u16 + 1),
                    y: area.bottom().saturating_sub(2),
                    width: hint.len() as u16,
                    height: 1,
                };
                frame.render_widget(
                    Paragraph::new(hint)
                        .style(Style::default().fg(t.muted).bg(t.background)),
                    hint_area,
                );
            }
        }
    }
}
