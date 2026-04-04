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

    // Compute threaded list once per frame and pass it to sub-renderers.
    // Previously threaded_comments() was called 4x per frame (render_comment_list,
    // render_detail x2, visible_comment_count). Now it runs exactly once.
    let threaded = threaded_comments(app);

    // Split body: list (40%) | detail (60%)
    let body_chunks = Layout::horizontal([
        Constraint::Percentage(40),
        Constraint::Percentage(60),
    ])
    .split(chunks[1]);

    render_comment_list(frame, app, body_chunks[0], &t, &threaded);
    render_detail(frame, app, body_chunks[1], &t, &threaded);

    let pane_hint = if app.double_check_pane == 0 {
        ("[Tab]", "→ Detail")
    } else {
        ("[Tab]", "← List")
    };

    let fix_label = format!("Fix with {}", app.config.llm.provider);

    keybind_bar::render(
        frame,
        chunks[2],
        &[
            ("[Esc]", "Back"),
            ("[jk]", "Nav"),
            ("[Space]", "Toggle"),
            pane_hint,
            ("[c]", "New comment"),
            ("[g]", "Edit comment"),
            ("[A]", "Approve all"),
            ("[D]", "Reject all"),
            ("[Del]", "Delete"),
            ("[P]", "Preview"),
            ("[r]", "Run missing"),
            ("[R]", "Restart"),
            ("[F]", &fix_label),
            ("[1-7]", "Filter agent"),
            ("[?]", "Help"),
        ],
        &t,
    );
}

fn render_header(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    // Count root-level comments only (no replies) so the count matches GitHub's "conversations".
    let (total, approved, rejected, pending) = match &app.draft {
        Some(d) => {
            let roots: Vec<_> = d.comments.iter().filter(|c| c.parent_github_id.is_none()).collect();
            (
                roots.len(),
                roots.iter().filter(|c| c.status == CommentStatus::Approved).count(),
                roots.iter().filter(|c| c.status == CommentStatus::Rejected).count(),
                roots.iter().filter(|c| c.status == CommentStatus::Pending).count(),
            )
        }
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
        .border_style(Style::default().fg(t.title))
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

/// Returns comments in threaded order: `(orig_idx, comment, depth)`.
/// depth=0 → root comment, depth=1 → reply.
/// Roots appear first; their replies follow immediately after them.
/// agent_filter=Some(n) shows only that agent (0 = manual/github).
pub fn threaded_comments<'a>(app: &'a App) -> Vec<(usize, &'a crate::review::models::GeneratedComment, usize)> {
    let draft = match &app.draft {
        Some(d) => d,
        None => return vec![],
    };

    // Apply agent filter
    let visible: Vec<(usize, &crate::review::models::GeneratedComment)> = draft
        .comments
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            if let Some(filter_idx) = app.agent_filter {
                match &c.source {
                    CommentSource::Agent { agent_id, .. } => {
                        let idx = app.agents.iter()
                            .position(|a| a.agent.id == *agent_id)
                            .map(|i| i as u8 + 1)
                            .unwrap_or(0);
                        return idx == filter_idx;
                    }
                    CommentSource::Manual | CommentSource::GithubReview { .. } => {
                        return filter_idx == 0;
                    }
                }
            }
            true
        })
        .collect();

    // Build set of github_ids present in the visible list
    let visible_github_ids: std::collections::HashSet<u64> = visible
        .iter()
        .filter_map(|(_, c)| c.github_id)
        .collect();

    // Separate roots from replies
    let (roots, replies): (Vec<_>, Vec<_>) = visible.into_iter().partition(|(_, c)| {
        c.parent_github_id
            .map(|pid| !visible_github_ids.contains(&pid))
            .unwrap_or(true)
    });

    // Build reply map: parent_github_id → sorted Vec<(orig_idx, comment)>
    let mut reply_map: std::collections::HashMap<u64, Vec<(usize, &crate::review::models::GeneratedComment)>> =
        std::collections::HashMap::new();
    for (orig_idx, c) in replies {
        if let Some(pid) = c.parent_github_id {
            reply_map.entry(pid).or_default().push((orig_idx, c));
        }
    }
    // Sort replies within each thread by created_at
    for v in reply_map.values_mut() {
        v.sort_by_key(|(_, c)| c.created_at);
    }

    // Build output: root then its replies
    let mut result = Vec::new();
    for (orig_idx, root) in roots {
        result.push((orig_idx, root, 0usize));
        if let Some(root_gh_id) = root.github_id {
            if let Some(children) = reply_map.get(&root_gh_id) {
                for (cidx, child) in children {
                    result.push((*cidx, *child, 1usize));
                }
            }
        }
    }
    result
}

/// How many visible (filtered + threaded) comments there are — for navigation bounds.
pub fn visible_comment_count(app: &App) -> usize {
    threaded_comments(app).len()
}

/// Return the comment at visual position `idx` together with its original draft index.
pub fn comment_at(app: &App, visual_idx: usize) -> Option<(usize, &crate::review::models::GeneratedComment)> {
    threaded_comments(app)
        .into_iter()
        .nth(visual_idx)
        .map(|(orig, c, _)| (orig, c))
}

fn render_comment_list<'a>(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    t: &Theme,
    comments: &'a [(usize, &'a crate::review::models::GeneratedComment, usize)],
) {
    let focused = app.double_check_pane == 0;
    let border_style = if focused {
        Style::default().fg(t.border_focused).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.border)
    };
    let border_color = if focused { t.border_focused } else { t.border };

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

    // Build reply-count map for root comments
    let reply_counts: std::collections::HashMap<u64, usize> = {
        let mut m = std::collections::HashMap::new();
        for (_, c, depth) in comments.iter() {
            if *depth > 0 {
                if let Some(pid) = c.parent_github_id {
                    *m.entry(pid).or_insert(0usize) += 1;
                }
            }
        }
        m
    };

    let items: Vec<ListItem> = comments
        .iter()
        .enumerate()
        .map(|(visual_i, (_, comment, depth))| {
            let selected = visual_i == app.double_check_selected;
            let is_reply = *depth > 0;

            let status_icon = match comment.status {
                CommentStatus::Approved => Span::styled("✓ ", Style::default().fg(t.agent_done)),
                CommentStatus::Rejected => Span::styled("✗ ", Style::default().fg(t.agent_failed)),
                CommentStatus::Pending  => Span::styled("○ ", Style::default().fg(t.muted)),
            };

            let sev_color = match comment.severity {
                Severity::Critical  => t.critical,
                Severity::Warning   => t.warning,
                Severity::Suggestion => t.suggestion,
                Severity::Praise    => t.praise,
            };

            let source = match &comment.source {
                CommentSource::Agent { agent_icon, agent_name, .. } => format!("{} {}", agent_icon, agent_name),
                CommentSource::Manual => "✍ manual".to_string(),
                CommentSource::GithubReview { user, state, .. } => format!("💬 {} ({})", user, state),
            };

            // Reply count suffix for root comments
            let reply_suffix = if !is_reply {
                comment.github_id
                    .and_then(|gid| reply_counts.get(&gid))
                    .map(|n| format!(" [{} repl{}]", n, if *n == 1 { "y" } else { "ies" }))
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let indent = if is_reply { "  ↳ " } else { "" };
            let file_info = match &comment.file_path {
                Some(f) => {
                    let short = if f.len() > 18 { format!("…{}", &f[f.len()-17..]) } else { f.clone() };
                    format!("{} {}:{}", indent, short, comment.line.unwrap_or(0))
                }
                None => indent.to_string(),
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
            let line2 = Line::from(vec![
                Span::styled(
                    format!("  {}{}", if is_reply { "  " } else { "" }, source),
                    Style::default().fg(t.muted),
                ),
                Span::styled(reply_suffix, Style::default().fg(t.warning)),
            ]);

            ListItem::new(vec![line1, line2]).style(row_style)
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(app.double_check_selected.min(comments.len().saturating_sub(1))));

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
        let pos = app.double_check_selected.min(max_s);
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

fn render_detail<'a>(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    t: &Theme,
    threaded: &'a [(usize, &'a crate::review::models::GeneratedComment, usize)],
) {
    let focused = app.double_check_pane == 1;
    let border_style = if focused {
        Style::default().fg(t.border_focused).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.border)
    };
    let border_color = if focused { t.border_focused } else { t.border };

    let selected_comment = threaded
        .get(app.double_check_selected)
        .map(|(_, c, _)| *c);

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
        CommentSource::GithubReview { user, state, .. } => format!("💬 GitHub Review — {} ({})", user, state),
    };

    let location_line = match (&comment.file_path, comment.line) {
        (Some(f), Some(l)) => format!("{}:{}", f, l),
        (Some(f), None)    => f.clone(),
        _                  => "(no file)".to_string(),
    };

    // Determine if this comment is part of a thread (root with replies, or a reply)
    let thread_root_gh_id: Option<u64> = if comment.parent_github_id.is_some() {
        comment.parent_github_id
    } else if comment.github_id.is_some()
        && threaded.iter().any(|(_, c, _)| c.parent_github_id == comment.github_id)
    {
        comment.github_id
    } else {
        None
    };

    // Collect the full thread (root + replies in order) when applicable
    let thread: Vec<&crate::review::models::GeneratedComment> = if let Some(root_id) = thread_root_gh_id {
        threaded
            .iter()
            .filter(|(_, c, _)| c.github_id == Some(root_id) || c.parent_github_id == Some(root_id))
            .map(|(_, c, _)| *c)
            .collect()
    } else {
        vec![]
    };

    let divider = Span::styled(
        "─".repeat(area.width.saturating_sub(4) as usize),
        Style::default().fg(t.border),
    );

    // Build lines for the detail panel
    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled(status_label.0, Style::default().fg(Color::Black).bg(status_label.1).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(format!("[{}]", comment.severity), Style::default().fg(sev_color).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled(source_line, Style::default().fg(t.muted))),
        Line::from(Span::styled(location_line, Style::default().fg(t.suggestion))),
        Line::from(divider.clone()),
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
                    if *kind != '-' {
                        spans.extend(syntax::highlight(content, ext, Some(row_bg)));
                    } else {
                        spans.push(Span::styled(
                            content.clone(),
                            Style::default().fg(t.agent_failed).bg(row_bg),
                        ));
                    }
                    lines.push(Line::from(spans));
                }
                lines.push(Line::from(divider.clone()));
                lines.push(Line::from(""));
            }
        }
    }

    // ── Thread conversation ────────────────────────────────────────────────────
    if !thread.is_empty() {
        let thread_count = thread.len();
        lines.push(Line::from(Span::styled(
            format!(" 💬 Thread ({} message{})", thread_count, if thread_count == 1 { "" } else { "s" }),
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));

        for tc in &thread {
            let is_selected = tc.id == comment.id;
            let is_reply = tc.parent_github_id.is_some();
            let indent = if is_reply { "    " } else { "" };
            let connector = if is_reply { "┗ " } else { "┌ " };

            let tc_author = match &tc.source {
                CommentSource::Agent { agent_icon, agent_name, .. } => format!("{} {}", agent_icon, agent_name),
                CommentSource::Manual => "✍ you".to_string(),
                CommentSource::GithubReview { user, .. } => user.clone(),
            };
            let tc_status_icon = match tc.status {
                CommentStatus::Approved => "✓",
                CommentStatus::Rejected => "✗",
                CommentStatus::Pending  => "○",
            };
            let tc_status_color = match tc.status {
                CommentStatus::Approved => t.agent_done,
                CommentStatus::Rejected => t.agent_failed,
                CommentStatus::Pending  => t.muted,
            };

            // Header row: connector + author + status
            let header_style = if is_selected {
                Style::default().fg(t.selected_fg).bg(t.selected_bg).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.title)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{}{}", indent, connector), Style::default().fg(t.border)),
                Span::styled(tc_author, header_style),
                Span::raw("  "),
                Span::styled(tc_status_icon, Style::default().fg(tc_status_color)),
            ]));

            // Body rows (indented)
            let body_indent = if is_reply { "      " } else { "  " };
            for body_line in tc.effective_body().lines() {
                lines.push(Line::from(Span::styled(
                    format!("{}{}", body_indent, body_line),
                    Style::default().fg(if is_selected { t.foreground } else { t.foreground }),
                )));
            }
            lines.push(Line::from(""));
        }
        lines.push(Line::from(divider.clone()));
    } else {
        // No thread — just show the comment body
        let body = comment.effective_body();
        for line in body.lines() {
            lines.push(Line::from(line.to_string()));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(divider.clone()));
    }

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
