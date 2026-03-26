use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::App;
use crate::tui::keybindings::InputMode;
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
        Constraint::Length(3),
    ])
    .split(area);

    render_header(frame, app, chunks[0], &t);
    render_editor(frame, app, chunks[1], &t);
    render_mode_indicator(frame, app, chunks[2], &t);

    let hint = if app.input_mode == InputMode::Insert {
        &[("[Esc]", "Normal mode"), ("[Enter]", "New line")][..]
    } else {
        &[
            ("[i]", "Insert"),
            ("[Esc]", "Back"),
            ("[Enter]", "Save"),
            ("[f]", "Files"),
        ][..]
    };

    keybind_bar::render(frame, chunks[3], hint, &t);
}

fn render_header(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
    let title = format!(" Compose Comment — PR #{pr_num} ");
    let block = Block::default()
        .title(title.as_str())
        .title_style(Style::default().fg(t.title).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border_focused))
        .style(Style::default().bg(t.background));
    frame.render_widget(block, area);
}

fn render_editor(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let focused = app.input_mode == InputMode::Insert;
    let border_color = if focused { t.border_focused } else { t.border };
    let title = if focused {
        " Editor — INSERT "
    } else {
        " Editor — NORMAL "
    };

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(if focused { t.warning } else { t.muted }))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.background));

    let content: Vec<Line> = if app.compose_text.is_empty() && !focused {
        vec![Line::from(Span::styled(
            "Press [i] to start writing your comment…",
            Style::default().fg(t.muted),
        ))]
    } else {
        let mut display = app.compose_text.clone();
        if focused {
            // Insert block cursor at byte position
            let pos = (0..=app.compose_cursor.min(display.len()))
                .rev()
                .find(|&p| display.is_char_boundary(p))
                .unwrap_or(0);
            display.insert(pos, '█');
        }
        // Split on newlines so each line renders as a separate TUI line
        display
            .split('\n')
            .map(|l| Line::from(l.to_string()))
            .collect()
    };

    let para = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(t.foreground).bg(t.background));

    frame.render_widget(para, area);
}

fn render_mode_indicator(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let (mode_text, mode_color) = match app.input_mode {
        InputMode::Normal => ("NORMAL", t.suggestion),
        InputMode::Insert => ("INSERT", t.warning),
    };

    let word_count = app.compose_text.split_whitespace().count();
    let char_count = app.compose_text.chars().count();

    let left = Span::styled(
        format!(" {} ", mode_text),
        Style::default()
            .fg(Color::Black)
            .bg(mode_color)
            .add_modifier(Modifier::BOLD),
    );
    let right = Span::styled(
        format!(" {char_count} chars, {word_count} words "),
        Style::default().fg(t.muted),
    );

    let line = Line::from(vec![left, Span::raw(" "), right]);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(t.background)),
        area,
    );
}
