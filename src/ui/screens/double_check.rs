use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::app::App;
use crate::review::models::{CommentSource, CommentStatus, Severity};
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

    render_header(frame, app, chunks[0], &t);
    render_comments(frame, app, chunks[1], &t);
    keybind_bar::render(
        frame,
        chunks[2],
        &[
            ("[Esc]", "Back"),
            ("[jk]", "Nav"),
            ("[Space]", "Toggle"),
            ("[c]", "New comment"),
            ("[A]", "Approve all"),
            ("[D]", "Reject all"),
            ("[P]", "Preview"),
            ("[1-7]", "Filter agent"),
            ("[?]", "Help"),
        ],
        &t,
    );
}

fn render_header(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let (total, approved, rejected) = match &app.draft {
        Some(d) => {
            let total = d.comments.len();
            let approved = d.comments.iter().filter(|c| c.status == CommentStatus::Approved).count();
            let rejected = d.comments.iter().filter(|c| c.status == CommentStatus::Rejected).count();
            (total, approved, rejected)
        }
        None => (0, 0, 0),
    };

    let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
    let filter_hint = match app.agent_filter {
        Some(n) => {
            let name = app.agents
                .get((n as usize).saturating_sub(1))
                .map(|a| a.agent.name.as_str())
                .unwrap_or("agent");
            format!(" [filter: {name}]")
        }
        None => String::new(),
    };

    let title = format!(" Double-Check Comments — PR #{pr_num}{filter_hint} ");
    let meta = format!(" {total} total | {approved} approved | {rejected} rejected ");

    let block = Block::default()
        .title(title.as_str())
        .title_style(Style::default().fg(t.title).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border_focused))
        .style(Style::default().bg(t.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(meta)
            .style(Style::default().fg(t.muted))
            .alignment(Alignment::Right),
        inner,
    );
}

fn render_comments(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let draft = match &app.draft {
        Some(d) => d,
        None => {
            frame.render_widget(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.border)),
                area,
            );
            return;
        }
    };

    let comments: Vec<_> = draft
        .comments
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            // Apply agent filter
            if let Some(filter_idx) = app.agent_filter {
                match &c.source {
                    CommentSource::Agent { agent_id, .. } => {
                        // Filter by agent index (1-7 → find by position)
                        let idx = app
                            .agents
                            .iter()
                            .position(|a| a.agent.id == *agent_id)
                            .map(|i| i as u8 + 1)
                            .unwrap_or(0);
                        if idx != filter_idx {
                            return false;
                        }
                    }
                    CommentSource::Manual => return filter_idx == 0,
                }
            }
            true
        })
        .collect();

    if comments.is_empty() {
        let msg = if app.draft.as_ref().map(|d| d.comments.is_empty()).unwrap_or(true) {
            "  No comments generated."
        } else {
            "  No comments match the current filter."
        };
        frame.render_widget(
            Paragraph::new(msg)
                .style(Style::default().fg(t.muted))
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(t.border))),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = comments
        .iter()
        .map(|(orig_i, comment)| {
            let selected = *orig_i == app.double_check_selected;
            let status_icon = match comment.status {
                CommentStatus::Approved => Span::styled("✓ ", Style::default().fg(t.agent_done)),
                CommentStatus::Rejected => Span::styled("✗ ", Style::default().fg(t.agent_failed)),
                CommentStatus::Pending => Span::styled("○ ", Style::default().fg(t.muted)),
            };

            let severity_color = match comment.severity {
                Severity::Critical => t.critical,
                Severity::Warning => t.warning,
                Severity::Suggestion => t.suggestion,
                Severity::Praise => t.praise,
            };
            let severity_span = Span::styled(
                format!("[{}] ", comment.severity),
                Style::default().fg(severity_color),
            );

            let source_span = match &comment.source {
                CommentSource::Agent { agent_name, agent_icon, .. } => {
                    Span::styled(
                        format!("{} {} ", agent_icon, agent_name),
                        Style::default().fg(t.muted),
                    )
                }
                CommentSource::Manual => {
                    Span::styled("✍ manual ", Style::default().fg(t.muted))
                }
            };

            let file_span = match &comment.file_path {
                Some(path) => Span::styled(
                    format!("{}:{} ", path, comment.line.unwrap_or(0)),
                    Style::default().fg(t.suggestion),
                ),
                None => Span::raw(""),
            };

            let body = comment.effective_body();
            let preview = if body.len() > 80 {
                format!("{}…", &body[..80])
            } else {
                body.to_string()
            };

            let row_style = if selected {
                Style::default().bg(t.selected_bg).fg(t.selected_fg)
            } else {
                Style::default().bg(t.background).fg(t.foreground)
            };

            let first_line = Line::from(vec![
                status_icon,
                severity_span,
                source_span,
                file_span,
            ]);
            let second_line = Line::from(Span::styled(
                format!("  {}", preview),
                Style::default().fg(if selected { t.selected_fg } else { t.foreground }),
            ));

            ListItem::new(vec![first_line, second_line]).style(row_style)
        })
        .collect();

    let mut list_state = ListState::default();
    let display_selected = comments
        .iter()
        .position(|(i, _)| *i == app.double_check_selected)
        .unwrap_or(0);
    list_state.select(Some(display_selected));

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.border))
                .style(Style::default().bg(t.background)),
        )
        .highlight_style(Style::default().bg(t.selected_bg).fg(t.selected_fg));

    frame.render_stateful_widget(list, area, &mut list_state);

    // Vertical scrollbar
    let total_items = comments.len();
    let visible_height = area.height.saturating_sub(2) as usize; // subtract borders
    if total_items > visible_height {
        let max_s = total_items.saturating_sub(visible_height);
        let pos = display_selected.min(max_s);
        let mut sb_state = ScrollbarState::new(max_s).position(pos);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"))
                .thumb_symbol("█")
                .track_symbol(Some("│")),
            area,
            &mut sb_state,
        );
    }
}
