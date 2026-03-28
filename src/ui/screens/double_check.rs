use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Wrap,
};

use crate::app::App;
use crate::review::models::{CommentSource, CommentStatus, Severity};
use crate::ui::components::{keybind_bar, syntax};
use crate::ui::theme::Theme;

// ── Diff context extraction ──────────────────────────────────────────────────

/// Parse the new-file start line from a unified diff hunk header.
/// Format: `@@ -old_start[,old_count] +new_start[,new_count] @@`
fn parse_hunk_new_start(hunk: &str) -> Option<u32> {
    let plus_part = hunk.split('+').nth(1)?;
    let num_str = plus_part.split([',', ' ']).next()?;
    num_str.parse().ok()
}

/// Extract diff lines around `target_line` (new-file line number) for `file_path`.
/// Returns `(new_file_line_or_0, kind [' '/'+'/ '-'], content)` tuples.
fn extract_diff_context(
    diff_lines: &[String],
    file_path: &str,
    target_line: u32,
    context: usize,
) -> Vec<(u32, char, String)> {
    let mut in_file = false;
    let mut new_line: u32 = 0;
    let mut collected: Vec<(u32, char, String)> = Vec::new();

    for line in diff_lines {
        if line.starts_with("diff --git ") {
            if !collected.is_empty() && in_file {
                break; // moved past our file
            }
            in_file = line.contains(&format!(" b/{}", file_path));
            continue;
        }
        if !in_file { continue; }
        if line.starts_with("--- ") || line.starts_with("+++ ") || line.starts_with("index ") {
            continue;
        }
        if line.starts_with("@@ ") {
            if let Some(start) = parse_hunk_new_start(line) {
                new_line = start;
            }
            continue;
        }
        let kind = line.chars().next().unwrap_or(' ');
        let content = if line.len() > 1 { line[1..].to_string() } else { String::new() };
        match kind {
            '+' => { collected.push((new_line, '+', content)); new_line += 1; }
            '-' => { collected.push((0, '-', content)); }
            ' ' => { collected.push((new_line, ' ', content)); new_line += 1; }
            _ => {}
        }
    }

    // Find the target line index, fallback to nearest
    let idx = collected.iter()
        .position(|(ln, _, _)| *ln == target_line)
        .or_else(|| collected.iter().position(|(ln, _, _)| *ln >= target_line))
        .unwrap_or(0);

    let start = idx.saturating_sub(context);
    let end = (idx + context + 1).min(collected.len());
    collected[start..end].to_vec()
}

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

    // Split body: list (40%) | detail (60%)
    let body_chunks = Layout::horizontal([
        Constraint::Percentage(40),
        Constraint::Percentage(60),
    ])
    .split(chunks[1]);

    render_comment_list(frame, app, body_chunks[0], &t);
    render_detail(frame, app, body_chunks[1], &t);

    let pane_hint = if app.double_check_pane == 0 {
        ("[Tab]", "→ Detail")
    } else {
        ("[Tab]", "← List")
    };

    keybind_bar::render(
        frame,
        chunks[2],
        &[
            ("[Esc]", "Back"),
            ("[jk]", "Nav"),
            ("[Space]", "Toggle"),
            pane_hint,
            ("[c]", "New comment"),
            ("[A]", "Approve all"),
            ("[D]", "Reject all"),
            ("[P]", "Preview"),
            ("[r]", "Run missing"),
            ("[R]", "Restart"),
            ("[C]", "→ Claude Code"),
            ("[1-7]", "Filter agent"),
            ("[?]", "Help"),
        ],
        &t,
    );
}

fn render_header(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let (total, approved, rejected, pending) = match &app.draft {
        Some(d) => (
            d.comments.len(),
            d.approved_count(),
            d.rejected_count(),
            d.pending_count(),
        ),
        None => (0, 0, 0, 0),
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

    let title = format!(" Double-Check — PR #{pr_num}{filter_hint} ");
    let meta = format!(
        " {total} total | ✓ {approved} approved | ○ {pending} pending | ✗ {rejected} rejected "
    );

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

fn filtered_comments<'a>(app: &'a App) -> Vec<(usize, &'a crate::review::models::GeneratedComment)> {
    let draft = match &app.draft {
        Some(d) => d,
        None => return vec![],
    };

    draft
        .comments
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            if let Some(filter_idx) = app.agent_filter {
                match &c.source {
                    CommentSource::Agent { agent_id, .. } => {
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
        .collect()
}

fn render_comment_list(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let focused = app.double_check_pane == 0;
    let border_style = if focused {
        Style::default().fg(t.border_focused).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.border)
    };
    let border_color = if focused { t.border_focused } else { t.border };

    let comments = filtered_comments(app);

    if comments.is_empty() {
        let msg = if app.draft.as_ref().map(|d| d.comments.is_empty()).unwrap_or(true) {
            "  No comments generated."
        } else {
            "  No comments match the current filter."
        };
        frame.render_widget(
            Paragraph::new(msg)
                .style(Style::default().fg(t.muted))
                .block(
                    Block::default()
                        .title(" Comments ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(border_color)),
                ),
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

            let sev_color = match comment.severity {
                Severity::Critical => t.critical,
                Severity::Warning => t.warning,
                Severity::Suggestion => t.suggestion,
                Severity::Praise => t.praise,
            };

            let source = match &comment.source {
                CommentSource::Agent { agent_icon, agent_name, .. } => {
                    format!("{} {}", agent_icon, agent_name)
                }
                CommentSource::Manual => "✍ manual".to_string(),
            };

            let file_info = match &comment.file_path {
                Some(f) => {
                    // Truncate long paths
                    let short = if f.len() > 20 {
                        format!("…{}", &f[f.len() - 19..])
                    } else {
                        f.clone()
                    };
                    format!(" {}:{}", short, comment.line.unwrap_or(0))
                }
                None => String::new(),
            };

            let row_style = if selected && focused {
                Style::default().bg(t.selected_bg).fg(t.selected_fg)
            } else {
                Style::default().bg(t.background).fg(t.foreground)
            };

            let line1 = Line::from(vec![
                status_icon,
                Span::styled(format!("[{}]", comment.severity), Style::default().fg(sev_color)),
                Span::styled(file_info, Style::default().fg(t.suggestion)),
            ]);
            let line2 = Line::from(Span::styled(
                format!("  {}", source),
                Style::default().fg(t.muted),
            ));

            ListItem::new(vec![line1, line2]).style(row_style)
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
                .title(if focused { " Comments [focused] " } else { " Comments " })
                .title_style(Style::default().fg(t.title))
                .borders(Borders::ALL)
                .border_style(border_style)
                .style(Style::default().bg(t.background)),
        )
        .highlight_style(Style::default().bg(t.selected_bg).fg(t.selected_fg));

    frame.render_stateful_widget(list, area, &mut list_state);

    let total_items = comments.len();
    let visible_height = area.height.saturating_sub(2) as usize;
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

fn render_detail(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let focused = app.double_check_pane == 1;
    let border_style = if focused {
        Style::default().fg(t.border_focused).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.border)
    };
    let border_color = if focused { t.border_focused } else { t.border };

    let comments = filtered_comments(app);
    let selected_comment = comments
        .iter()
        .find(|(i, _)| *i == app.double_check_selected)
        .map(|(_, c)| *c);

    let comment = match selected_comment {
        Some(c) => c,
        None => {
            frame.render_widget(
                Paragraph::new("  Select a comment to see its full content.")
                    .style(Style::default().fg(t.muted))
                    .block(
                        Block::default()
                            .title(" Detail ")
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(border_color)),
                    ),
                area,
            );
            return;
        }
    };

    let sev_color = match comment.severity {
        Severity::Critical => t.critical,
        Severity::Warning => t.warning,
        Severity::Suggestion => t.suggestion,
        Severity::Praise => t.praise,
    };

    let status_label = match comment.status {
        CommentStatus::Approved => (" ✓ APPROVED ", t.agent_done),
        CommentStatus::Rejected => (" ✗ REJECTED ", t.agent_failed),
        CommentStatus::Pending  => (" ○ PENDING  ", t.muted),
    };

    let source_line = match &comment.source {
        CommentSource::Agent { agent_name, agent_icon, .. } => {
            format!("{} {}", agent_icon, agent_name)
        }
        CommentSource::Manual => "✍ Manual".to_string(),
    };

    let location_line = match (&comment.file_path, comment.line) {
        (Some(f), Some(l)) => format!("{}:{}", f, l),
        (Some(f), None)    => f.clone(),
        _                  => "(no file)".to_string(),
    };

    // Build lines for the detail panel
    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled(status_label.0, Style::default().fg(Color::Black).bg(status_label.1).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(format!("[{}]", comment.severity), Style::default().fg(sev_color).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled(source_line, Style::default().fg(t.muted))),
        Line::from(Span::styled(location_line, Style::default().fg(t.suggestion))),
        Line::from(Span::styled(
            "─".repeat(area.width.saturating_sub(4) as usize),
            Style::default().fg(t.border),
        )),
        Line::from(""),
    ];

    // Code context block — shown when the comment is tied to a specific file+line
    if let (Some(file_path), Some(target_line)) = (&comment.file_path, comment.line) {
        if let Some(diff_lines) = &app.diff_lines_cache {
            let ctx = extract_diff_context(diff_lines, file_path, target_line, 3);
            if !ctx.is_empty() {
                let ext = file_path.rsplit('.').next(); // e.g. "rs", "ts", "py"

                // File header
                lines.push(Line::from(vec![
                    Span::styled(" 📄 ", Style::default().fg(t.muted)),
                    Span::styled(
                        format!("{}:{}", file_path, target_line),
                        Style::default().fg(t.suggestion).add_modifier(Modifier::ITALIC),
                    ),
                ]));

                // Code lines with syntax highlighting
                for (ln, kind, content) in &ctx {
                    let is_target = *ln == target_line && *kind != '-';
                    let (gutter_fg, row_bg, marker) = if is_target {
                        (t.warning, t.selected_bg, "▶")
                    } else {
                        match kind {
                            '+' => (t.agent_done,   Color::Rgb(0, 30, 0), "+"),
                            '-' => (t.agent_failed, Color::Rgb(30, 0, 0), "-"),
                            _   => (t.muted,        t.background, " "),
                        }
                    };
                    let ln_str = if *ln > 0 { format!("{:4}", ln) } else { "    ".to_string() };

                    // Build line: gutter | line-number | highlighted code
                    let mut spans = vec![
                        Span::styled(
                            format!(" {} ", marker),
                            Style::default().fg(gutter_fg).bg(row_bg),
                        ),
                        Span::styled(
                            format!("{} ", ln_str),
                            Style::default().fg(t.muted).bg(row_bg),
                        ),
                    ];
                    // Only apply syntax highlight for context/add lines (not removed lines)
                    if *kind != '-' {
                        let bg_override = Some(row_bg);
                        spans.extend(syntax::highlight(content, ext, bg_override));
                    } else {
                        spans.push(Span::styled(
                            content.clone(),
                            Style::default().fg(t.agent_failed).bg(row_bg),
                        ));
                    }
                    lines.push(Line::from(spans));
                }
                lines.push(Line::from(Span::styled(
                    "─".repeat(area.width.saturating_sub(4) as usize),
                    Style::default().fg(t.border),
                )));
                lines.push(Line::from(""));
            }
        }
    }

    // Add full comment body — split into lines
    let body = comment.effective_body();
    for line in body.lines() {
        lines.push(Line::from(line.to_string()));
    }
    lines.push(Line::from(""));

    // Add hint at bottom
    lines.push(Line::from(Span::styled(
        "[Space] toggle status   [Tab] back to list",
        Style::default().fg(t.muted).add_modifier(Modifier::ITALIC),
    )));

    let total_lines = lines.len();
    let scroll = app.double_check_detail_scroll;
    let inner_h = area.height.saturating_sub(2) as usize;

    let block = Block::default()
        .title(if focused { " Comment Detail [focused] " } else { " Comment Detail " })
        .title_style(Style::default().fg(t.title))
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().bg(t.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0))
        .style(Style::default().fg(t.foreground).bg(t.background));
    frame.render_widget(para, inner);

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
