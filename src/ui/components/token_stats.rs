use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::App;

/// Render a token-statistics overlay centered over the screen.
pub fn render_stats(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let overlay = centered_rect(60, 50, area);

    frame.render_widget(Clear, overlay);

    let bg = Color::Rgb(10, 20, 10);
    let accent = Color::Rgb(80, 220, 120);
    let gold = Color::Rgb(255, 200, 50);
    let muted = Color::Rgb(120, 140, 120);
    let white = Color::Rgb(230, 230, 230);

    let block = Block::default()
        .title(" ◈ Token Statistics ")
        .title_style(Style::default().fg(accent).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent))
        .style(Style::default().bg(bg));

    let inner = block.inner(overlay);
    frame.render_widget(block, overlay);

    let total_in = app.token_input_total;
    let total_out = app.token_output_total;
    let total_calls = app.token_calls_total;
    let total_tokens = total_in + total_out;

    // Cost estimate — Claude Sonnet pricing (approximate)
    // Input: $3 / 1M tokens → $0.000003 per token
    // Output: $15 / 1M tokens → $0.000015 per token
    let cost_usd = (total_in as f64 * 3.0 + total_out as f64 * 15.0) / 1_000_000.0;

    let avg_in = if total_calls > 0 { total_in / total_calls } else { 0 };
    let avg_out = if total_calls > 0 { total_out / total_calls } else { 0 };

    let lines: Vec<Line> = vec![
        Line::from(Span::styled("  Session Summary", Style::default().fg(accent).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  ─────────────────────────────────────────────────  ", Style::default().fg(muted)),
        ]),
        Line::from(vec![
            Span::styled("  Agent calls      ", Style::default().fg(muted)),
            Span::styled(format!("{}", total_calls), Style::default().fg(white).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  Input tokens     ", Style::default().fg(muted)),
            Span::styled(format_tokens(total_in), Style::default().fg(gold).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  Output tokens    ", Style::default().fg(muted)),
            Span::styled(format_tokens(total_out), Style::default().fg(gold).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  Total tokens     ", Style::default().fg(muted)),
            Span::styled(format_tokens(total_tokens), Style::default().fg(accent).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Per-call Averages", Style::default().fg(accent).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  ─────────────────────────────────────────────────  ", Style::default().fg(muted)),
        ]),
        Line::from(vec![
            Span::styled("  Avg input        ", Style::default().fg(muted)),
            Span::styled(format_tokens(avg_in), Style::default().fg(white)),
        ]),
        Line::from(vec![
            Span::styled("  Avg output       ", Style::default().fg(muted)),
            Span::styled(format_tokens(avg_out), Style::default().fg(white)),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Cost Estimate (Claude Sonnet pricing)", Style::default().fg(accent).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  ─────────────────────────────────────────────────  ", Style::default().fg(muted)),
        ]),
        Line::from(vec![
            Span::styled("  Estimated cost   ", Style::default().fg(muted)),
            Span::styled(format!("~${:.4}", cost_usd), Style::default().fg(Color::Rgb(255, 160, 60)).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  Input rate       ", Style::default().fg(muted)),
            Span::styled("$3.00 / 1M tokens", Style::default().fg(muted)),
        ]),
        Line::from(vec![
            Span::styled("  Output rate      ", Style::default().fg(muted)),
            Span::styled("$15.00 / 1M tokens", Style::default().fg(muted)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Note: ", Style::default().fg(muted).add_modifier(Modifier::ITALIC)),
            Span::styled("Token counts are estimates (chars / 4).", Style::default().fg(muted).add_modifier(Modifier::ITALIC)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  [Esc] / [T]  Close",
            Style::default().fg(Color::Rgb(80, 100, 80)),
        )),
    ];

    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .style(Style::default().bg(bg));
    frame.render_widget(para, inner);
}

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_width = r.width * percent_x / 100;
    let popup_height = r.height * percent_y / 100;
    let x = r.x + (r.width.saturating_sub(popup_width)) / 2;
    let y = r.y + (r.height.saturating_sub(popup_height)) / 2;
    Rect {
        x,
        y,
        width: popup_width.max(1),
        height: popup_height.max(1),
    }
}
