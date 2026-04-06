use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{App, Screen};
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
        Constraint::Length(1),
        Constraint::Min(3),
    ])
    .split(area);

    render_header(frame, app, chunks[0], &t);
    render_editor(frame, app, chunks[1], &t);
    render_mode_indicator(frame, app, chunks[2], &t);

    keybind_bar::render(
        frame,
        chunks[3],
        &[
            ("[Esc]", "Normal/Back"),
            ("[i]", "Insert"),
            ("[Enter]", "Save/Post"),
            ("[s]", "Suggestion"),
        ],
        &t,
    );
}

fn render_header(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let title = if app.compose_quick_mode {
        " 📝 Quick Comment — Posts directly to PR conversation "
    } else {
        " ✍️ Review Comment — Saved to your review session "
    };

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(if app.compose_quick_mode { t.warning } else { t.title }).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.compose_quick_mode { t.warning } else { t.border_focused }))
        .style(Style::default().bg(t.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(path) = &app.compose_file_path {
        let line = app.compose_line.unwrap_or(0);
        let loc = format!(" @ {}:{} ", path, line);
        frame.render_widget(
            Paragraph::new(loc).style(Style::default().fg(t.muted)).alignment(Alignment::Right),
            inner,
        );
    }
}

fn render_editor(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let focused = app.input_mode == InputMode::Insert;
    let border_color = if focused { t.border_focused } else { t.border };
    let title = if focused { " Editor — INSERT " } else { " Editor — NORMAL " };

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(if focused { t.warning } else { t.muted }))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.background));

    // Use internal textarea for rendering
    let mut widget = app.compose_editor.textarea.clone();
    widget.set_block(block);
    widget.set_cursor_style(if focused { Style::default().bg(Color::White).fg(Color::Black) } else { Style::default().add_modifier(Modifier::HIDDEN) });

    frame.render_widget(&widget, area);
}

fn render_mode_indicator(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let (mode_text, mode_color) = match app.input_mode {
        InputMode::Normal => ("NORMAL", t.suggestion),
        InputMode::Insert => ("INSERT", t.warning),
    };

    let content = app.compose_editor.get_text();
    let word_count = content.split_whitespace().count();
    let char_count = content.chars().count();

    let left = Span::styled(
        format!(" {} ", mode_text),
        Style::default().fg(Color::Black).bg(mode_color).add_modifier(Modifier::BOLD),
    );
    let right = Span::styled(
        format!(" {char_count} chars, {word_count} words "),
        Style::default().fg(t.muted),
    );

    let line = Line::from(vec![left, Span::raw(" "), right]);
    frame.render_widget(Paragraph::new(line).style(Style::default().bg(t.background)), area);
}
