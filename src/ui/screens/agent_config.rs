use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

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

    let body = Layout::horizontal([
        Constraint::Percentage(35),
        Constraint::Percentage(65),
    ])
    .split(chunks[1]);

    render_agent_list(frame, app, body[0], &t);
    render_agent_detail(frame, app, body[1], &t);

    keybind_bar::render(
        frame,
        chunks[2],
        &[
            ("[Esc]", "Back"),
            ("[jk]", "Nav"),
            ("[Space]", "Toggle enabled"),
        ],
        &t,
    );
}

fn render_header(frame: &mut Frame, _app: &App, area: Rect, t: &Theme) {
    let block = Block::default()
        .title(" Agent Configuration ")
        .title_style(Style::default().fg(t.title).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border_focused))
        .style(Style::default().bg(t.background));
    frame.render_widget(block, area);
}

fn render_agent_list(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let items: Vec<ListItem> = app
        .agents
        .iter()
        .enumerate()
        .map(|(i, def)| {
            let selected = i == app.agent_config_selected;
            let enabled_mark = if def.agent.enabled {
                Span::styled("● ", Style::default().fg(t.agent_done))
            } else {
                Span::styled("○ ", Style::default().fg(t.agent_disabled))
            };
            let name_style = if selected {
                Style::default().bg(t.selected_bg).fg(t.selected_fg)
            } else {
                Style::default().fg(t.foreground)
            };
            let line = Line::from(vec![
                enabled_mark,
                Span::styled(def.agent.icon.as_str(), Style::default()),
                Span::raw(" "),
                Span::styled(def.agent.name.as_str(), name_style),
            ]);
            ListItem::new(line)
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.agent_config_selected));

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Agents ")
                .title_style(Style::default().fg(t.title))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.border))
                .style(Style::default().bg(t.background)),
        )
        .highlight_style(Style::default().bg(t.selected_bg).fg(t.selected_fg));

    frame.render_stateful_widget(list, area, &mut state);
}

fn render_agent_detail(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let def = match app.agents.get(app.agent_config_selected) {
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

    let enabled_text = if def.agent.enabled { "Yes" } else { "No" };
    let llm_override = def
        .agent
        .llm
        .as_ref()
        .map(|l| {
            format!(
                "model={} temp={} max_tokens={}",
                l.model.as_deref().unwrap_or("default"),
                l.temperature.map(|f| f.to_string()).unwrap_or("default".into()),
                l.max_tokens.map(|n| n.to_string()).unwrap_or("default".into()),
            )
        })
        .unwrap_or_else(|| "default".into());

    let text = format!(
        "ID: {}\nName: {}\nDescription: {}\nEnabled: {}\nOrder: {}\n\nLLM Override:\n{}\n\nContext:\n  diff={} description={} ticket={} files={}\n  exclude: {}\n  include: {}",
        def.agent.id,
        def.agent.name,
        def.agent.description,
        enabled_text,
        def.agent.order,
        llm_override,
        def.agent.context.include_diff,
        def.agent.context.include_pr_description,
        def.agent.context.include_ticket,
        def.agent.context.include_file_list,
        def.agent.context.exclude_patterns.join(", "),
        def.agent.context.include_patterns.join(", "),
    );

    let agent_title = format!(" {} {} ", def.agent.icon, def.agent.name);
    let para = Paragraph::new(text)
        .block(
            Block::default()
                .title(agent_title.as_str())
                .title_style(Style::default().fg(t.title))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.border))
                .style(Style::default().bg(t.background)),
        )
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(t.foreground));

    frame.render_widget(para, area);
}
