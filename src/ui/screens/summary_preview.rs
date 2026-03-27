use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

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

    keybind_bar::render(
        frame,
        chunks[3],
        &[
            ("[Esc]", "Back"),
            ("[←→]", "Review type"),
            ("[Enter/p]", "Submit to GitHub"),
            ("[q]", "Abort"),
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
    let body = app
        .draft
        .as_ref()
        .and_then(|d| d.review_body.as_deref())
        .unwrap_or("(Press [P] from Double-Check to generate the review summary from your approved comments.)");

    let para = Paragraph::new(body)
        .block(
            Block::default()
                .title(" Review Body ")
                .title_style(Style::default().fg(t.title))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.border))
                .style(Style::default().bg(t.background)),
        )
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(t.foreground));

    frame.render_widget(para, area);
}

fn render_comment_list(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
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
                Span::styled(
                    format!("[{}] ", c.severity),
                    Style::default().fg(sev_color),
                ),
                Span::styled(format!("{}{} ", source, file_info), Style::default().fg(t.muted)),
            ]);
            let line2 = Line::from(Span::styled(
                format!("  {}", body_preview),
                Style::default().fg(t.foreground),
            ));
            ListItem::new(vec![line1, line2])
        })
        .collect();

    let comments_title = format!(" Approved Comments ({}) ", approved.len());
    let list = List::new(items).block(
        Block::default()
            .title(comments_title.as_str())
            .title_style(Style::default().fg(t.title))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border))
            .style(Style::default().bg(t.background)),
    );

    frame.render_widget(list, area);
}

fn render_event_selector(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let block = Block::default()
        .title(" Review Type — choose what kind of GitHub review to submit ")
        .title_style(Style::default().fg(t.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border_focused))
        .style(Style::default().bg(t.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let events_line = Line::from(
        EVENTS
            .iter()
            .enumerate()
            .flat_map(|(i, ev)| {
                let label = ev.as_github_str();
                let selected = i == app.summary_event_idx;
                let radio = if selected { "(●) " } else { "(○) " };
                let style = if selected {
                    Style::default()
                        .fg(t.selected_fg)
                        .bg(t.selected_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(t.foreground)
                };
                vec![
                    Span::styled(format!(" {}{}", radio, label), style),
                    Span::styled("  ", Style::default()),
                ]
            })
            .collect::<Vec<_>>(),
    );

    frame.render_widget(Paragraph::new(events_line), inner);
}
