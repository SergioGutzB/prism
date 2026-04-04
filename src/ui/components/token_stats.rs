use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};
use crate::app::{App, ModelStats};
use crate::ui::theme::Theme;

fn range_label(range: u8) -> &'static str {
    match range {
        0 => "Last 7 Days",
        1 => "Last 15 Days",
        2 => "Last 30 Days",
        _ => "All Time",
    }
}

fn days_for_range(range: u8) -> Option<i64> {
    match range {
        0 => Some(7),
        1 => Some(15),
        2 => Some(30),
        _ => None,
    }
}

/// Filter model stats by an already-computed cutoff date string ("YYYY-MM-DD").
/// Pass `None` to sum all time.
fn filtered_stats(stats: &ModelStats, cutoff: Option<&str>) -> (u64, u64, u64) {
    match cutoff {
        None => (stats.calls, stats.input_tokens, stats.output_tokens),
        Some(cutoff_str) => {
            stats.daily.iter()
                .filter(|(day, _)| day.as_str() >= cutoff_str)
                .fold((0u64, 0u64, 0u64), |acc, (_, ds)| {
                    (acc.0 + ds.calls, acc.1 + ds.input_tokens, acc.2 + ds.output_tokens)
                })
        }
    }
}

pub fn render_stats(frame: &mut Frame, app: &App) {
    let t = Theme::current(&app.config.ui.theme);
    let area = frame.area();

    let w = (area.width * 70 / 100).max(60).min(area.width);
    let h = (area.height * 70 / 100).max(15).min(area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let popup_area = Rect { x, y, width: w, height: h };

    frame.render_widget(Clear, popup_area);

    let days = days_for_range(app.stats_range);
    let label = range_label(app.stats_range);

    // Compute the cutoff string once — used for every model in the loop below.
    // Previously this was recomputed inside filtered_stats() for every model.
    let cutoff_string = days.map(|d| {
        (chrono::Utc::now() - chrono::Duration::days(d))
            .format("%Y-%m-%d")
            .to_string()
    });
    let cutoff = cutoff_string.as_deref();

    let mut items = vec![
        ListItem::new(Line::from(vec![
            Span::styled(" Range: ", Style::default().fg(t.muted)),
            Span::styled(label, Style::default().fg(t.warning).add_modifier(Modifier::BOLD)),
            Span::styled("   [← →] cycle range", Style::default().fg(t.muted)),
        ])),
        ListItem::new(Line::from(Span::raw(""))),
    ];

    let mut total_calls = 0u64;
    let mut total_in = 0u64;
    let mut total_out = 0u64;

    // Sort models for stable display
    let mut models: Vec<_> = app.model_stats.iter().collect();
    models.sort_by_key(|(k, _)| k.as_str());

    for (model, stats) in &models {
        let (calls, input_tokens, output_tokens) = filtered_stats(stats, cutoff);
        total_calls += calls;
        total_in += input_tokens;
        total_out += output_tokens;

        let since = stats.start_date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "unknown".to_string());

        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!(" 🤖 {:20}", model), Style::default().fg(t.title).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!(" calls:{:5} | in:{:9} | out:{:9}", calls, input_tokens, output_tokens),
                Style::default().fg(t.foreground),
            ),
            Span::styled(format!(" (since {})", since), Style::default().fg(t.muted)),
        ])));
    }

    items.push(ListItem::new(Line::from(Span::styled(
        " ".repeat(w as usize),
        Style::default().add_modifier(Modifier::UNDERLINED).fg(t.border),
    ))));

    items.push(ListItem::new(Line::from(vec![
        Span::styled(" TOTALS:             ", Style::default().fg(t.muted)),
        Span::styled(
            format!(" calls:{:5} | in:{:9} | out:{:9}", total_calls, total_in, total_out),
            Style::default().fg(t.suggestion).add_modifier(Modifier::BOLD),
        ),
    ])));

    items.push(ListItem::new(Line::from(Span::raw(""))));
    items.push(ListItem::new(Line::from(
        Span::styled(" [← →] Change Range   [Esc] or [T] Close", Style::default().fg(t.muted)),
    )));

    let list = List::new(items).block(
        Block::default()
            .title(" LLM Usage Statistics ")
            .title_style(Style::default().fg(t.title).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .style(Style::default().bg(t.background)),
    );

    frame.render_widget(list, popup_area);
}
