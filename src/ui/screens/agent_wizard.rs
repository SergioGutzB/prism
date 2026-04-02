use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{App, AgentWizardField, Screen};
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
    render_form(frame, app, chunks[1], &t);

    keybind_bar::render(
        frame,
        chunks[2],
        &[
            ("[Esc]", "Back"),
            ("[Tab]", "Next Field"),
            ("[i]", "Enter Insert"),
            ("[Enter]", "Save Agent"),
        ],
        &t,
    );
}

fn render_header(frame: &mut Frame, _app: &App, area: Rect, t: &Theme) {
    let block = Block::default()
        .title(" Custom Agent Creation Wizard ")
        .title_style(Style::default().fg(t.title).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.title))
        .style(Style::default().bg(t.background));
    frame.render_widget(block, area);
}

fn render_form(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // ID
        Constraint::Length(3), // Name
        Constraint::Length(3), // Icon
        Constraint::Min(0),    // System Prompt
    ])
    .margin(1)
    .split(area);

    // 1. Agent ID
    let id_style = if app.wizard_field == AgentWizardField::Id {
        Style::default().fg(t.title).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.foreground)
    };
    let id_border = if app.wizard_field == AgentWizardField::Id {
        t.border_focused
    } else {
        t.border
    };
    let id_para = Paragraph::new(app.wizard_id.as_str())
        .block(Block::default().title(" Agent ID (unique_slug) ").borders(Borders::ALL).border_style(Style::default().fg(id_border)))
        .style(id_style);
    frame.render_widget(id_para, chunks[0]);

    // 2. Name
    let name_style = if app.wizard_field == AgentWizardField::Name {
        Style::default().fg(t.title).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.foreground)
    };
    let name_border = if app.wizard_field == AgentWizardField::Name {
        t.border_focused
    } else {
        t.border
    };
    let name_para = Paragraph::new(app.wizard_name.as_str())
        .block(Block::default().title(" Display Name ").borders(Borders::ALL).border_style(Style::default().fg(name_border)))
        .style(name_style);
    frame.render_widget(name_para, chunks[1]);

    // 3. Icon
    let icon_style = if app.wizard_field == AgentWizardField::Icon {
        Style::default().fg(t.title).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.foreground)
    };
    let icon_border = if app.wizard_field == AgentWizardField::Icon {
        t.border_focused
    } else {
        t.border
    };
    let icon_para = Paragraph::new(app.wizard_icon.as_str())
        .block(Block::default().title(" Icon (emoji) ").borders(Borders::ALL).border_style(Style::default().fg(icon_border)))
        .style(icon_style);
    frame.render_widget(icon_para, chunks[2]);

    // 4. System Prompt
    let prompt_border = if app.wizard_field == AgentWizardField::SystemPrompt {
        t.border_focused
    } else {
        t.border
    };
    
    let block = Block::default()
        .title(" System Prompt (instructions for the AI) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(prompt_border));
    
    // Create a copy of the textarea widget to render
    let mut widget = app.wizard_prompt_editor.textarea.clone();
    widget.set_block(block);
    
    frame.render_widget(&widget, chunks[3]);
}
