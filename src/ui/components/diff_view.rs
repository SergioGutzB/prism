use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::App;
use crate::ui::theme::Theme;

/// Colorize a single diff line. Called only for visible lines.
fn colorize_line(raw: &str, t: &Theme) -> Line<'static> {
    let line = raw.to_string();
    let style = if line.starts_with("@@") {
        Style::default().fg(t.diff_hunk)
    } else if line.starts_with('+') && !line.starts_with("+++") {
        Style::default().fg(t.diff_add)
    } else if line.starts_with('-') && !line.starts_with("---") {
        Style::default().fg(t.diff_remove)
    } else if line.starts_with("diff ")
        || line.starts_with("index ")
        || line.starts_with("---")
        || line.starts_with("+++")
    {
        Style::default().fg(t.title)
    } else {
        Style::default().fg(t.diff_context)
    };
    Line::from(Span::styled(line, style))
}

/// Render the diff panel using the pre-split line cache.
/// Only colorizes the lines that fit in the visible area — O(height), not O(total).
pub fn render(frame: &mut Frame, app: &App, area: Rect, t: &Theme, focused: bool) {
    let border_color = if focused { t.border_focused } else { t.border };

    let title = match app.selected_pane {
        1 => " Diff [focused] ",
        _ => " Diff ",
    };

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(t.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Loading state
    if app.diff_loading || (app.current_diff.is_none() && app.pr_loading) {
        frame.render_widget(
            Paragraph::new(format!("{} Loading diff…", app.spinner_char()))
                .style(Style::default().fg(t.loading)),
            inner,
        );
        return;
    }

    let lines = match &app.diff_lines_cache {
        Some(l) => l,
        None => {
            frame.render_widget(
                Paragraph::new("No diff available.")
                    .style(Style::default().fg(t.muted)),
                inner,
            );
            return;
        }
    };

    let total = lines.len();
    if total == 0 {
        frame.render_widget(
            Paragraph::new("(empty diff)").style(Style::default().fg(t.muted)),
            inner,
        );
        return;
    }

    // Clamp scroll so we never go past the last line
    let max_scroll = total.saturating_sub(inner.height as usize);
    let scroll = (app.diff_scroll as usize).min(max_scroll);

    // Only colorize the visible slice — O(height) instead of O(total)
    let visible: Vec<Line> = lines
        .iter()
        .skip(scroll)
        .take(inner.height as usize)
        .map(|raw| colorize_line(raw, t))
        .collect();

    frame.render_widget(
        Paragraph::new(visible).style(Style::default().bg(t.background)),
        inner,
    );

    // Scroll indicator: line position, not a loading percentage
    if total > inner.height as usize {
        let current_line = scroll + 1;
        let indicator = format!(" {}/{} ", current_line, total);
        let ind_width = indicator.len() as u16;
        let ind_area = Rect {
            x: area.right().saturating_sub(ind_width + 1),
            y: area.bottom().saturating_sub(1),
            width: ind_width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(indicator)
                .style(Style::default().fg(t.muted).bg(t.background)),
            ind_area,
        );
    }
}

/// Legacy function kept for compatibility — no longer called each frame.
/// Use the diff_lines_cache in App instead.
pub fn parse_diff_lines(diff: &str, _theme: &str) -> Vec<String> {
    diff.lines().map(str::to_string).collect()
}
