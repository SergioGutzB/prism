pub mod components;
pub mod editor;
pub mod screens;
pub mod theme;

use ratatui::Frame;

use crate::app::App;
use crate::ui::screens::{
    agent_config, agent_runner, agent_wizard, claude_output, double_check, file_tree, pr_detail, pr_list,
    review_compose, setup, summary_preview,
};

/// Top-level render dispatch — pure function, never modifies App.
pub fn render(frame: &mut Frame, app: &App) {
    match &app.screen {
        crate::app::Screen::Setup => setup::render(frame, app),
        crate::app::Screen::PrList => pr_list::render(frame, app),
        crate::app::Screen::PrDetail => pr_detail::render(frame, app),
        crate::app::Screen::FileTree => file_tree::render(frame, app),
        crate::app::Screen::ReviewCompose => review_compose::render(frame, app),
        crate::app::Screen::AgentRunner => agent_runner::render(frame, app),
        crate::app::Screen::DoubleCheck => double_check::render(frame, app),
        crate::app::Screen::SummaryPreview => summary_preview::render(frame, app),
        crate::app::Screen::AgentConfig => agent_config::render(frame, app),
        crate::app::Screen::AgentWizard => agent_wizard::render(frame, app),
        crate::app::Screen::Settings => render_settings(frame, app),
        crate::app::Screen::ClaudeCodeOutput => claude_output::render(frame, app),
    }

    // Overlay popup on top of everything
    if app.popup.is_some() {
        components::popup::render_popup(frame, app);
    }

    // Help and stats overlays (above popup)
    if app.show_help {
        components::help::render_help(frame, app);
    }
    if app.show_stats {
        components::token_stats::render_stats(frame, app);
    }
}

fn render_settings(frame: &mut Frame, app: &App) {
    use ratatui::prelude::*;
    use ratatui::widgets::{Block, Borders, List, ListItem};
    use theme::Theme;

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

    // Header
    let header = Block::default()
        .title(" PRISM \u{2014} Settings ")
        .title_style(Style::default().fg(t.title).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border_focused))
        .style(Style::default().bg(t.background));
    frame.render_widget(header, chunks[0]);

    // Build settings list
    let cfg = &app.config;
    let gh_status = if cfg.is_github_configured() {
        Span::styled("\u{2713} configured", Style::default().fg(t.agent_done))
    } else {
        Span::styled("\u{2717} not configured", Style::default().fg(t.agent_failed))
    };
    let llm_status = if cfg.is_llm_configured() {
        Span::styled("\u{2713} available", Style::default().fg(t.agent_done))
    } else {
        Span::styled("\u{2717} unavailable", Style::default().fg(t.agent_failed))
    };
    let user_str = app.github_user.as_deref().unwrap_or("unknown").to_string();

    let token_display = if cfg.github.token.is_empty() {
        "(not set)".to_string()
    } else {
        let token = &cfg.github.token;
        let prefix_len = 4.min(token.len());
        let suffix_start = token.len().saturating_sub(4);
        format!("{}\u{2026}{}", &token[..prefix_len], &token[suffix_start..])
    };

    let items: Vec<ListItem> = vec![
        // Section: GitHub
        ListItem::new(Line::from(Span::styled(" \u{2500}\u{2500} GitHub \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}", Style::default().fg(t.title)))),
        ListItem::new(Line::from(vec![
            Span::styled("   Owner/Repo   ", Style::default().fg(t.muted)),
            Span::styled(format!("{}/{}", cfg.github.owner, cfg.github.repo), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("   Token        ", Style::default().fg(t.muted)),
            Span::styled(token_display, Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("   Status       ", Style::default().fg(t.muted)),
            gh_status,
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("   Logged in as ", Style::default().fg(t.muted)),
            Span::styled(user_str, Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        ])),
        // Section: LLM
        ListItem::new(Line::from(Span::styled(" \u{2500}\u{2500} LLM \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}", Style::default().fg(t.title)))),
        ListItem::new(Line::from(vec![
            Span::styled("   Provider     ", Style::default().fg(t.muted)),
            Span::styled(cfg.llm.provider.clone(), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("   Model        ", Style::default().fg(t.muted)),
            Span::styled(cfg.llm.model.clone(), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("   Status       ", Style::default().fg(t.muted)),
            llm_status,
        ])),
        // Section: Agents
        ListItem::new(Line::from(Span::styled(" \u{2500}\u{2500} Agents \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}", Style::default().fg(t.title)))),
        ListItem::new(Line::from(vec![
            Span::styled("   Agents dir   ", Style::default().fg(t.muted)),
            Span::styled(cfg.agents.agents_dir.clone(), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("   Concurrency  ", Style::default().fg(t.muted)),
            Span::styled(cfg.agents.concurrency.to_string(), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("   Timeout      ", Style::default().fg(t.muted)),
            Span::styled(format!("{}s", cfg.agents.timeout_secs), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("   Review rigor ", Style::default().fg(t.muted)),
            Span::styled(cfg.agents.review_rigor.clone(), Style::default().fg(t.warning).add_modifier(Modifier::BOLD)),
        ])),
        // Section: UI
        ListItem::new(Line::from(Span::styled(" \u{2500}\u{2500} UI \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}", Style::default().fg(t.title)))),
        ListItem::new(Line::from(vec![
            Span::styled("   Theme        ", Style::default().fg(t.muted)),
            Span::styled(cfg.ui.theme.clone(), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("   Syntax hl    ", Style::default().fg(t.muted)),
            Span::styled(cfg.ui.highlight_syntax.to_string(), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("   Line numbers ", Style::default().fg(t.muted)),
            Span::styled(cfg.ui.show_line_numbers.to_string(), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        ])),
        // Section: Publishing
        ListItem::new(Line::from(Span::styled(" \u{2500}\u{2500} Publishing \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}", Style::default().fg(t.title)))),
        ListItem::new(Line::from(vec![
            Span::styled("   Confirm publish  ", Style::default().fg(t.muted)),
            Span::styled(cfg.publishing.confirm_before_publish.to_string(), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("   Auto-translate   ", Style::default().fg(t.muted)),
            Span::styled(cfg.publishing.auto_translate_to_english.to_string(), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("   Auto-correct     ", Style::default().fg(t.muted)),
            Span::styled(cfg.publishing.auto_correct_grammar.to_string(), Style::default().fg(t.foreground).add_modifier(Modifier::BOLD)),
        ])),
    ];

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border))
            .style(Style::default().bg(t.background)),
    );
    frame.render_widget(list, chunks[1]);

    // Keybind bar
    components::keybind_bar::render(
        frame,
        chunks[2],
        &[("[Esc]", "Back"), ("[L]", "Reload"), ("[?]", "Help")],
        &t,
    );
}
