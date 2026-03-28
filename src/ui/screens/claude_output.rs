use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};

use crate::app::App;
use crate::ui::components::keybind_bar;
use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, app: &App) {
    let t = Theme::current(&app.config.ui.theme);
    let area = frame.area();

    frame.render_widget(
        Block::default().style(Style::default().bg(t.background)),
        area,
    );

    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(3),
    ])
    .split(area);

    // ── Header ───────────────────────────────────────────────────────────────
    let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
    let title = if app.claude_output_loading {
        format!(" {} Claude Code — Processing PR #{} … ", app.spinner_char(), pr_num)
    } else {
        format!(" ✦ Claude Code — PR #{} ", pr_num)
    };

    frame.render_widget(
        Block::default()
            .title(title)
            .title_style(
                Style::default()
                    .fg(t.title)
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(
                Style::default()
                    .fg(t.border_focused)
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(t.background)),
        chunks[0],
    );

    // ── Content ───────────────────────────────────────────────────────────────
    let content_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border))
        .style(Style::default().bg(t.background));

    let inner = content_block.inner(chunks[1]);
    frame.render_widget(content_block, chunks[1]);

    if app.claude_output_loading && app.claude_output.is_empty() {
        let spinner = Paragraph::new(format!(
            "\n\n  {} Sending review comments to Claude Code…",
            app.spinner_char()
        ))
        .style(Style::default().fg(t.loading));
        frame.render_widget(spinner, inner);
    } else {
        let scroll = app.claude_output_scroll;
        let lines: Vec<Line> = app.claude_output.lines()
            .map(|l| Line::from(l.to_string()))
            .collect();
        let total_lines = lines.len();
        let inner_h = inner.height as usize;

        let para = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll as u16, 0))
            .style(Style::default().fg(t.foreground).bg(t.background));
        frame.render_widget(para, inner);

        if total_lines > inner_h {
            let max_s = total_lines.saturating_sub(inner_h);
            let pos = scroll.min(max_s);
            let mut sb = ScrollbarState::new(max_s).position(pos);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(Some("▲"))
                    .end_symbol(Some("▼"))
                    .thumb_symbol("█")
                    .track_symbol(Some("│")),
                chunks[1],
                &mut sb,
            );
        }
    }

    // ── Keybind bar ───────────────────────────────────────────────────────────
    let retry_hint = if !app.claude_output_loading && !app.claude_fix_prompt.is_empty() {
        vec![
            ("[Esc]", "Back"),
            ("[jk]", "Scroll"),
            ("[G/gg]", "Bottom/Top"),
            ("[C]", "Retry"),
        ]
    } else {
        vec![
            ("[Esc]", "Back"),
            ("[jk]", "Scroll"),
            ("[G/gg]", "Bottom/Top"),
        ]
    };
    keybind_bar::render(frame, chunks[2], &retry_hint, &t);
}
