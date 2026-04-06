use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Wrap,
};

use crate::app::{App, FixTaskStatus};
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
        Constraint::Min(3),
    ])
    .split(area);

    render_header(frame, app, chunks[0], &t);
    render_body(frame, app, chunks[1], &t);
    render_keybinds(frame, app, chunks[2], &t);
}

// ── Header ────────────────────────────────────────────────────────────────────

fn render_header(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
    let total = app.fix_tasks.len();
    let done = app.fix_tasks.iter()
        .filter(|t| matches!(t.status, FixTaskStatus::Done))
        .count();
    let failed = app.fix_tasks.iter()
        .filter(|t| matches!(t.status, FixTaskStatus::Failed(_)))
        .count();

    let status = if app.ai_fix_loading {
        let running_name = app.fix_tasks.iter()
            .find(|t| matches!(t.status, FixTaskStatus::Running))
            .map(|t| format!(" — applying fix {}/{}…", t.index, total))
            .unwrap_or_default();
        format!(" {} Claude Fix — PR #{pr_num}{running_name} ", app.spinner_char())
    } else if failed > 0 {
        format!(" ✦ Claude Fix — PR #{pr_num}  {done} done  {failed} failed ")
    } else {
        format!(" ✦ Claude Fix — PR #{pr_num}  {done}/{total} done ")
    };

    frame.render_widget(
        Block::default()
            .title(status)
            .title_style(Style::default().fg(t.title).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border_focused).add_modifier(Modifier::BOLD))
            .style(Style::default().bg(t.background)),
        area,
    );
}

// ── Body: task list (left) + output panel (right) ────────────────────────────

fn render_body(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    if app.fix_tasks.is_empty() {
        // Nothing running yet — show a simple loading spinner
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border))
            .style(Style::default().bg(t.background));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(format!("\n\n  {} Preparing tasks…", app.spinner_char()))
                .style(Style::default().fg(t.loading)),
            inner,
        );
        return;
    }

    let panes = Layout::horizontal([
        Constraint::Percentage(35),
        Constraint::Percentage(65),
    ])
    .split(area);

    render_task_list(frame, app, panes[0], t);
    render_task_output(frame, app, panes[1], t);
}

fn render_task_list(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let spinner = app.spinner_char().to_string();

    let items: Vec<ListItem> = app.fix_tasks.iter().map(|task| {
        let (icon, style) = match &task.status {
            FixTaskStatus::Pending  => ("○", Style::default().fg(t.muted)),
            FixTaskStatus::Running  => (spinner.as_str(), Style::default().fg(t.agent_running)),
            FixTaskStatus::Done     => ("✓", Style::default().fg(t.agent_done)),
            FixTaskStatus::Failed(_)=> ("✗", Style::default().fg(t.agent_failed)),
        };

        let selected = task.index == app.fix_tasks.get(app.fix_task_selected)
            .map(|t| t.index).unwrap_or(0);

        let row_style = if selected {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let line = Line::from(vec![
            Span::styled(format!(" {icon} "), style),
            Span::styled(
                format!("{:2}. {}", task.index, truncate(&task.summary, 26)),
                Style::default()
                    .fg(if selected { t.title } else { t.foreground })
                    .add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() }),
            ),
        ]).style(row_style);

        ListItem::new(line)
    }).collect();

    let block = Block::default()
        .title(format!(" Tasks ({}) ", app.fix_tasks.len()))
        .title_style(Style::default().fg(t.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border))
        .style(Style::default().bg(t.background));

    let mut list_state = ListState::default();
    list_state.select(Some(app.fix_task_selected));

    frame.render_stateful_widget(List::new(items).block(block), area, &mut list_state);
}

fn render_task_output(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let task = app.fix_tasks.get(app.fix_task_selected);

    // Avoid cloning the output string — borrow it directly for rendering.
    let spinner_str;
    let (title, content): (String, &str) = match task {
        None => ("Output".to_string(), ""),
        Some(t_info) => {
            let title = format!(" {} @ {} ", t_info.source, t_info.location);
            let content: &str = if t_info.output.is_empty() {
                if matches!(t_info.status, FixTaskStatus::Running) {
                    spinner_str = format!("  {} Waiting for Claude…", app.spinner_char());
                    &spinner_str
                } else if matches!(t_info.status, FixTaskStatus::Pending) {
                    "  Queued"
                } else {
                    ""
                }
            } else {
                &t_info.output
            };
            (title, content)
        }
    };

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(t.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border))
        .style(Style::default().bg(t.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Count lines without allocating — only needed for scroll bounds and scrollbar.
    let total_lines = content.lines().count();
    let inner_h = inner.height as usize;
    let scroll = app.ai_fix_scroll.min(total_lines.saturating_sub(1));

    // Paragraph accepts &str directly — no per-line String allocation.
    frame.render_widget(
        Paragraph::new(content)
            .wrap(Wrap { trim: false })
            .scroll((scroll as u16, 0))
            .style(Style::default().fg(t.foreground).bg(t.background)),
        inner,
    );

    if total_lines > inner_h {
        let max_s = total_lines.saturating_sub(inner_h);
        let mut sb = ScrollbarState::new(max_s).position(scroll.min(max_s));
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"))
                .thumb_symbol("█")
                .track_symbol(Some("│")),
            area,
            &mut sb,
        );
    }
}

// ── Keybind bar ───────────────────────────────────────────────────────────────

fn render_keybinds(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let selected_failed = app.fix_tasks.get(app.fix_task_selected)
        .map(|t| matches!(t.status, FixTaskStatus::Failed(_)))
        .unwrap_or(false);

    let mut hints: Vec<(&str, &str)> = vec![
        ("[Esc]", "Back"),
        ("[j/k]", "Navigate"),
    ];
    if total_lines_for_scroll(app) > 0 {
        hints.push(("[J/K]", "Scroll output"));
    }
    if selected_failed && !app.ai_fix_loading {
        hints.push(("[C]", "Retry task"));
    }
    keybind_bar::render(frame, area, &hints, t);
}

fn total_lines_for_scroll(app: &App) -> usize {
    app.fix_tasks.get(app.fix_task_selected)
        .map(|t| t.output.lines().count())
        .unwrap_or(0)
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}
