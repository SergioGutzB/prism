pub mod components;
pub mod screens;
pub mod theme;

use ratatui::Frame;

use crate::app::App;
use crate::ui::screens::{
    agent_config, agent_runner, double_check, file_tree, pr_detail, pr_list, review_compose,
    setup, summary_preview,
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
        crate::app::Screen::Settings => render_settings(frame, app),
    }

    // Overlay popup on top of everything
    if app.popup.is_some() {
        components::popup::render_popup(frame, app);
    }
}

fn render_settings(frame: &mut Frame, app: &App) {
    use ratatui::prelude::*;
    use ratatui::widgets::{Block, Borders, Paragraph};
    use theme::Theme;
    let t = Theme::current(&app.config.ui.theme);
    let area = frame.area();
    let block = Block::default()
        .title(" PRISM — Settings ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border_focused))
        .style(Style::default().bg(t.background).fg(t.foreground));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let text = Paragraph::new("Settings screen — coming soon.\n\nPress [Esc] to go back.")
        .style(Style::default().fg(t.foreground));
    frame.render_widget(text, inner);
}
