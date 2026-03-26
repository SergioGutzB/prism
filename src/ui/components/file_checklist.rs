use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use crate::review::models::ReviewDraft;
use crate::ui::theme::Theme;

/// Render the file checklist from a ReviewDraft into the given area.
pub fn render(
    frame: &mut Frame,
    draft: &ReviewDraft,
    area: Rect,
    t: &Theme,
    selected: usize,
) {
    let items: Vec<ListItem> = draft
        .file_checklist
        .iter()
        .enumerate()
        .map(|(i, (path, checked))| {
            let mark = if *checked {
                Span::styled("[✓] ", Style::default().fg(t.agent_done))
            } else {
                Span::styled("[ ] ", Style::default().fg(t.muted))
            };
            let comment_count = draft
                .comments
                .iter()
                .filter(|c| c.file_path.as_deref() == Some(path.as_str()))
                .count();
            let comments_span = if comment_count > 0 {
                Span::styled(
                    format!(" ({} comments)", comment_count),
                    Style::default().fg(t.suggestion),
                )
            } else {
                Span::raw("")
            };

            let row_style = if i == selected {
                Style::default().bg(t.selected_bg).fg(t.selected_fg)
            } else {
                Style::default().bg(t.background).fg(t.foreground)
            };

            ListItem::new(Line::from(vec![
                mark,
                Span::styled(path.as_str(), Style::default()),
                comments_span,
            ]))
            .style(row_style)
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(selected));

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Files ")
                .title_style(Style::default().fg(t.title))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.border))
                .style(Style::default().bg(t.background)),
        )
        .highlight_style(Style::default().bg(t.selected_bg).fg(t.selected_fg));

    frame.render_stateful_widget(list, area, &mut list_state);
}
