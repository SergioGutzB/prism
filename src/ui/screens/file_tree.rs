use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Gauge, List, ListItem, ListState, Paragraph, Row, Table, TableState};

use crate::app::App;
use crate::review::models::{CommentSource, CommentStatus, Severity};
use crate::ui::components::{diff_view, keybind_bar};
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
        Constraint::Length(3),
    ])
    .split(area);

    render_header(frame, app, chunks[0], &t);

    if app.file_tree_fullscreen && app.file_tree_pane == 1 {
        // Fullscreen: the detail panel takes the full body area
        render_detail(frame, app, chunks[1], &t);
    } else {
        // Normal split: file list left (38%), detail panel right (62%)
        let body = Layout::horizontal([
            Constraint::Percentage(38),
            Constraint::Percentage(62),
        ])
        .split(chunks[1]);

        render_table(frame, app, body[0], &t);
        render_detail(frame, app, body[1], &t);
    }

    render_progress(frame, app, chunks[2], &t);

    if app.file_tree_pane == 0 {
        keybind_bar::render(
            frame,
            chunks[3],
            &[
                ("[Esc]", "Back"),
                ("[jk]", "Navigate files"),
                ("[Enter]", "Jump to diff"),
                ("[→]", "View detail"),
                ("[x]", "Toggle check"),
            ],
            &t,
        );
    } else if app.file_tree_fullscreen {
        let split_hint = if app.file_tree_split { ("[Z]", "Unified") } else { ("[Z]", "Split") };
        keybind_bar::render(
            frame,
            chunks[3],
            &[
                ("[←/Esc]", "Exit full"),
                ("[jk]", "Scroll"),
                ("[c]", "Comment"),
                ("[z]", "Exit full"),
                split_hint,
            ],
            &t,
        );
    } else {
        keybind_bar::render(
            frame,
            chunks[3],
            &[
                ("[←]", "Back to files"),
                ("[jk]", "Navigate lines"),
                ("[c]", "Comment line"),
                ("[J/K]", "Scroll detail"),
                ("[z]", "Fullscreen"),
                ("[Z]", "Split"),
            ],
            &t,
        );
    }
}

fn render_header(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
    let file_count = app.draft.as_ref().map(|d| d.file_checklist.len()).unwrap_or(0);
    let title = format!(" File Tree — PR #{pr_num} ({file_count} files) ");
    let block = Block::default()
        .title(title.as_str())
        .title_style(Style::default().fg(t.title).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.title))
        .style(Style::default().bg(t.background));
    frame.render_widget(block, area);
}

fn render_table(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let draft = match &app.draft {
        Some(d) => d,
        None => {
            frame.render_widget(
                Paragraph::new("  No files loaded.\n  Open a PR and navigate here with [f].")
                    .style(Style::default().fg(t.muted))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(t.border)),
                    ),
                area,
            );
            return;
        }
    };

    let header_cells = ["", "File", "Cmts"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(t.title).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1);

    let rows: Vec<Row> = draft
        .file_checklist
        .iter()
        .enumerate()
        .map(|(i, (path, checked))| {
            let selected = i == app.file_tree_line;
            let check = if *checked { "✓" } else { " " };
            let comment_count = draft
                .comments
                .iter()
                .filter(|c| c.file_path.as_deref() == Some(path.as_str()))
                .count();
            let row_style = if selected {
                Style::default().bg(t.selected_bg).fg(t.selected_fg)
            } else {
                Style::default().bg(t.background).fg(t.foreground)
            };
            // Shorten long paths to show just the filename + parent dir
            let display_path = shorten_path(path);
            Row::new(vec![
                Cell::from(check).style(Style::default().fg(t.agent_done)),
                Cell::from(display_path),
                Cell::from(comment_count.to_string())
                    .style(Style::default().fg(if comment_count > 0 { t.warning } else { t.muted })),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Min(20),
            Constraint::Length(4),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if app.file_tree_pane == 0 { t.border_focused } else { t.border })),
    );

    let mut state = TableState::default();
    state.select(Some(app.file_tree_line));
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_detail(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let draft = match &app.draft {
        Some(d) => d,
        None => {
            frame.render_widget(
                Block::default()
                    .title(" File Detail ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.border))
                    .style(Style::default().bg(t.background)),
                area,
            );
            return;
        }
    };

    let selected_path = draft.file_checklist.keys().nth(app.file_tree_line);
    let path = match selected_path {
        Some(p) => p,
        None => {
            frame.render_widget(
                Block::default()
                    .title(" File Detail ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.border))
                    .style(Style::default().bg(t.background)),
                area,
            );
            return;
        }
    };

    let file_comments: Vec<_> = draft
        .comments
        .iter()
        .filter(|c| c.file_path.as_deref() == Some(path.as_str()))
        .collect();

    // In fullscreen mode hide the comments panel to maximize diff space
    let comment_height = if file_comments.is_empty() || app.file_tree_fullscreen {
        0u16
    } else {
        (file_comments.len() as u16 * 3).min(area.height / 3)
    };

    let detail_chunks = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(comment_height),
    ])
    .split(area);

    let diff_lines = extract_file_diff_lines(app, path);
    let cursor = Some(app.file_tree_scroll);
    let focused = app.file_tree_pane == 1;

    // Split mode (fullscreen only) uses the side-by-side renderer
    if app.file_tree_fullscreen && app.file_tree_split {
        diff_view::render_split_lines(
            frame,
            detail_chunks[0],
            &diff_lines,
            cursor,
            path,
            None,
            t,
            focused,
        );
    } else {
        // Both fullscreen-unified and non-fullscreen use render_unified_lines
        diff_view::render_unified_lines(
            frame,
            detail_chunks[0],
            &diff_lines,
            cursor,
            path,
            None,
            t,
            focused,
        );
    }

    // Comments section (hidden in fullscreen)
    if comment_height > 0 {
        let items: Vec<ListItem> = file_comments
            .iter()
            .map(|c| {
                let sev_color = match c.severity {
                    Severity::Critical => t.critical,
                    Severity::Warning => t.warning,
                    Severity::Suggestion => t.suggestion,
                    Severity::Praise => t.praise,
                };
                let status_icon = match c.status {
                    CommentStatus::Approved => Span::styled("✓ ", Style::default().fg(t.agent_done)),
                    CommentStatus::Rejected => Span::styled("✗ ", Style::default().fg(t.agent_failed)),
                    CommentStatus::Pending => Span::styled("○ ", Style::default().fg(t.muted)),
                };
                let agent = match &c.source {
                    CommentSource::Agent { agent_name, .. } => agent_name.as_str(),
                    CommentSource::Manual => "manual",
                    CommentSource::GithubReview { user, .. } => user.as_str(),
                };
                let line1 = Line::from(vec![
                    status_icon,
                    Span::styled(format!("[{}] ", c.severity), Style::default().fg(sev_color)),
                    Span::styled(
                        format!("line:{} {}", c.line.unwrap_or(0), agent),
                        Style::default().fg(t.muted),
                    ),
                ]);
                let body = c.effective_body();
                let preview = if body.len() > 70 { format!("{}…", &body[..70]) } else { body.to_string() };
                let line2 = Line::from(Span::styled(
                    format!("  {}", preview),
                    Style::default().fg(t.foreground),
                ));
                ListItem::new(vec![line1, line2])
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .title(format!(" Comments ({}) ", file_comments.len()))
                .title_style(Style::default().fg(t.title))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.border)),
        );
        frame.render_widget(list, detail_chunks[1]);
    }
}

fn render_progress(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let (total, checked) = match &app.draft {
        Some(d) => {
            let total = d.file_checklist.len();
            let checked = d.file_checklist.values().filter(|&&v| v).count();
            (total, checked)
        }
        None => (0, 0),
    };

    let ratio = if total > 0 { checked as f64 / total as f64 } else { 0.0 };
    let label = format!(" {checked}/{total} files reviewed ");
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).border_style(
            Style::default().fg(t.border),
        ))
        .gauge_style(Style::default().fg(t.agent_done).bg(t.background))
        .label(label)
        .ratio(ratio);

    frame.render_widget(gauge, area);
}

/// Extract diff lines for a specific file from app's full diff cache.
fn extract_file_diff_lines(app: &App, path: &str) -> Vec<String> {
    let diff = match &app.current_diff {
        Some(d) => d,
        None => return Vec::new(),
    };
    let target = format!("diff --git a/{} b/{}", path, path);
    let mut result = Vec::new();
    let mut found = false;

    for line in diff.lines() {
        if line == target {
            found = true;
        } else if found && line.starts_with("diff --git ") {
            break;
        }
        if found {
            result.push(line.to_string());
        }
    }
    result
}


fn shorten_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() <= 2 {
        path.to_string()
    } else {
        format!("…/{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
    }
}
