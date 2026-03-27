use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};

use crate::app::App;
use crate::review::models::{CommentSource, CommentStatus, ReviewEvent, Severity};
use crate::ui::components::keybind_bar;
use crate::ui::theme::Theme;

const EVENTS: &[ReviewEvent] = &[
    ReviewEvent::Comment,
    ReviewEvent::RequestChanges,
    ReviewEvent::Approve,
];

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
        Constraint::Length(5), // review event selector
        Constraint::Length(3),
    ])
    .split(area);

    render_header(frame, app, chunks[0], &t);

    let body_chunks = Layout::horizontal([
        Constraint::Percentage(40),
        Constraint::Percentage(60),
    ])
    .split(chunks[1]);

    render_body(frame, app, body_chunks[0], &t);
    render_comment_list(frame, app, body_chunks[1], &t);
    render_event_selector(frame, app, chunks[2], &t);

    let pane_hint = if app.summary_pane == 0 {
        ("[Tab]", "→ Comments")
    } else {
        ("[Tab]", "→ Body")
    };

    keybind_bar::render(
        frame,
        chunks[3],
        &[
            ("[Esc]", "Back"),
            ("[←→]", "Review type"),
            pane_hint,
            ("[jk]", "Scroll"),
            ("[g]", "Generate body"),
            ("[Enter/p]", "Submit"),
        ],
        &t,
    );
}

fn render_header(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
    let n = app
        .draft
        .as_ref()
        .map(|d| d.approved_count())
        .unwrap_or(0);

    let header_title = format!(" Summary Preview — PR #{pr_num} — {n} approved comments ");
    let block = Block::default()
        .title(header_title.as_str())
        .title_style(Style::default().fg(t.title).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border_focused))
        .style(Style::default().bg(t.background));
    frame.render_widget(block, area);
}

fn render_body(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let focused = app.summary_pane == 0;
    let border_color = if focused { t.border_focused } else { t.border };

    let body = app
        .draft
        .as_ref()
        .and_then(|d| d.review_body.as_deref())
        .unwrap_or("(Empty — press [g] to auto-generate from approved comments, or leave blank to submit only inline comments.)");

    let total_lines = body.lines().count().max(1);
    let scroll = app.summary_body_scroll;

    let para = Paragraph::new(body)
        .block(
            Block::default()
                .title(if focused { " Review Body [focused] " } else { " Review Body " })
                .title_style(Style::default().fg(t.title))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(t.background)),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0))
        .style(Style::default().fg(t.foreground));

    frame.render_widget(para, area);

    let inner_h = area.height.saturating_sub(2) as usize;
    if total_lines > inner_h {
        let max_s = total_lines.saturating_sub(inner_h);
        let mut sb_state = ScrollbarState::new(max_s).position(scroll.min(max_s));
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

fn render_comment_list(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let focused = app.summary_pane == 1;
    let border_color = if focused { t.border_focused } else { t.border };

    let draft = match &app.draft {
        Some(d) => d,
        None => {
            frame.render_widget(
                Block::default()
                    .title(" Approved Comments ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color)),
                area,
            );
            return;
        }
    };

    let approved: Vec<_> = draft
        .comments
        .iter()
        .filter(|c| c.status == CommentStatus::Approved)
        .collect();

    let items: Vec<ListItem> = approved
        .iter()
        .map(|c| {
            let sev_color = match c.severity {
                Severity::Critical => t.critical,
                Severity::Warning => t.warning,
                Severity::Suggestion => t.suggestion,
                Severity::Praise => t.praise,
            };
            let source = match &c.source {
                CommentSource::Agent { agent_name, .. } => agent_name.as_str(),
                CommentSource::Manual => "manual",
            };
            let file_info = match &c.file_path {
                Some(f) => format!(" {}:{}", f, c.line.unwrap_or(0)),
                None => String::new(),
            };
            let body_preview = {
                let b = c.effective_body();
                if b.len() > 60 { format!("{}…", &b[..60]) } else { b.to_string() }
            };
            let line1 = Line::from(vec![
                Span::styled(format!("[{}] ", c.severity), Style::default().fg(sev_color)),
                Span::styled(format!("{}{} ", source, file_info), Style::default().fg(t.muted)),
            ]);
            let line2 = Line::from(Span::styled(
                format!("  {}", body_preview),
                Style::default().fg(t.foreground),
            ));
            ListItem::new(vec![line1, line2])
        })
        .collect();

    let comments_title = format!(
        " {} Approved Comments ({}) ",
        if focused { "[focused] " } else { "" },
        approved.len()
    );

    let total = approved.len();
    let visible_h = area.height.saturating_sub(2) as usize;
    let scroll = app.summary_comments_scroll.min(total.saturating_sub(visible_h / 2));

    let list = List::new(items)
        .block(
            Block::default()
                .title(comments_title.as_str())
                .title_style(Style::default().fg(t.title))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(t.background)),
        )
        .scroll_padding(scroll);

    frame.render_widget(list, area);

    if total > visible_h {
        let max_s = total.saturating_sub(visible_h);
        let mut sb_state = ScrollbarState::new(max_s).position(scroll.min(max_s));
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

fn render_event_selector(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let block = Block::default()
        .title(" Review Type — [←→] to change ")
        .title_style(Style::default().fg(t.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border_focused))
        .style(Style::default().bg(t.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let is_own_pr = app.github_user.as_deref()
        .zip(app.current_pr.as_ref().map(|p| p.author.as_str()))
        .map(|(u, a)| u == a)
        .unwrap_or(false);

    let events_line = Line::from(
        EVENTS
            .iter()
            .enumerate()
            .flat_map(|(i, ev)| {
                let label = ev.as_github_str();
                let selected = i == app.summary_event_idx;
                let unavailable = i == 1 && is_own_pr; // REQUEST_CHANGES on own PR
                let radio = if selected { "(●) " } else { "(○) " };
                let display_label = if unavailable {
                    format!("{} ⚠ own PR", label)
                } else {
                    label.to_string()
                };
                let style = if unavailable {
                    Style::default().fg(t.warning)
                } else if selected {
                    Style::default()
                        .fg(t.selected_fg)
                        .bg(t.selected_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(t.foreground)
                };
                vec![
                    Span::styled(format!(" {}{}", radio, display_label), style),
                    Span::styled("    ", Style::default()),
                ]
            })
            .collect::<Vec<_>>(),
    );

    frame.render_widget(Paragraph::new(events_line), inner);
}
