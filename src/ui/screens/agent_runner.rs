use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::agents::models::AgentStatus;
use crate::app::App;
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
    render_agent_list(frame, app, chunks[1], &t);
    keybind_bar::render(frame, chunks[2], &[("[Esc]", "Cancel"), ("[q]", "Quit")], &t);
}

fn render_header(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
    let running = app
        .agent_statuses
        .values()
        .filter(|s| matches!(s, AgentStatus::Running { .. }))
        .count();
    let done = app
        .agent_statuses
        .values()
        .filter(|s| matches!(s, AgentStatus::Done { .. }))
        .count();
    let total = app.agents.len();

    let title = format!(" Agent Runner — PR #{pr_num} ");
    let status = if running > 0 {
        format!(" {} Running agents… {done}/{total} done ", app.spinner_char())
    } else {
        format!(" {done}/{total} completed ")
    };

    let block = Block::default()
        .title(title.as_str())
        .title_style(Style::default().fg(t.title).add_modifier(Modifier::BOLD))
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border_focused))
        .style(Style::default().bg(t.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(status)
            .style(Style::default().fg(t.loading))
            .alignment(Alignment::Right),
        inner,
    );
}

fn render_agent_list(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let spinner = app.spinner_char().to_string();

    let items: Vec<ListItem> = app
        .agents
        .iter()
        .map(|def| {
            let id = &def.agent.id;
            let name = &def.agent.name;
            let icon = &def.agent.icon;
            let status = app.agent_statuses.get(id);

            let status_icon: &str = match status {
                None | Some(AgentStatus::Pending) => "○",
                Some(AgentStatus::Disabled) => "─",
                Some(AgentStatus::Running { .. }) => spinner.as_str(),
                Some(AgentStatus::Done { .. }) => "✓",
                Some(AgentStatus::Failed { .. }) => "✗",
                Some(AgentStatus::Skipped { .. }) => "⊘",
            };

            let status_style = match status {
                None | Some(AgentStatus::Pending) => Style::default().fg(t.muted),
                Some(AgentStatus::Disabled) => Style::default().fg(t.agent_disabled),
                Some(AgentStatus::Running { .. }) => Style::default().fg(t.agent_running),
                Some(AgentStatus::Done { .. }) => Style::default().fg(t.agent_done),
                Some(AgentStatus::Failed { .. }) => Style::default().fg(t.agent_failed),
                Some(AgentStatus::Skipped { .. }) => Style::default().fg(t.agent_skipped),
            };

            let comment_info = match status {
                Some(AgentStatus::Done { comments, elapsed_ms }) => {
                    format!(" — {} comment(s) in {}ms", comments.len(), elapsed_ms)
                }
                Some(AgentStatus::Failed { error }) => {
                    format!(" — Error: {}", truncate(error, 40))
                }
                Some(AgentStatus::Skipped { reason }) => {
                    format!(" — {}", reason)
                }
                Some(AgentStatus::Running { started_at }) => {
                    let elapsed = chrono::Utc::now()
                        .signed_duration_since(*started_at)
                        .num_milliseconds();
                    format!(" — {}ms", elapsed)
                }
                _ => String::new(),
            };

            let line = Line::from(vec![
                Span::styled(format!(" {} ", status_icon), status_style),
                Span::styled(format!("{} ", icon), Style::default()),
                Span::styled(name.as_str(), Style::default().fg(t.foreground)),
                Span::styled(comment_info, Style::default().fg(t.muted)),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border))
            .style(Style::default().bg(t.background)),
    );

    frame.render_widget(list, area);
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}
