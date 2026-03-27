use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::App;
use crate::tui::keybindings::InputMode;
use crate::ui::components::{keybind_bar, syntax};
use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, app: &App) {
    let t = Theme::current(&app.config.ui.theme);
    let area = frame.area();

    frame.render_widget(
        Block::default().style(Style::default().bg(t.background)),
        area,
    );

    let has_context = app.compose_file_path.is_some() && !app.compose_context.is_empty();
    let hint: &[(&str, &str)] = if app.input_mode == InputMode::Insert {
        &[("[Esc]", "Normal mode"), ("[Enter]", "New line")]
    } else if app.compose_quick_mode {
        &[
            ("[i]", "Insert"),
            ("[Esc]", "Cancel"),
            ("[Enter]", "Publish comment"),
        ]
    } else if has_context {
        &[
            ("[i]", "Insert"),
            ("[s]", "Suggestion block"),
            ("[Esc]", "Back"),
            ("[Enter]", "Add to Review"),
            ("[v]", "View Reviews"),
        ]
    } else {
        &[
            ("[i]", "Insert"),
            ("[Esc]", "Back"),
            ("[Enter]", "Add to Review"),
            ("[v]", "View Reviews"),
        ]
    };

    if has_context {
        let context_height = (app.compose_context.len() as u16 + 2).min(10);
        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(context_height),
            Constraint::Min(0),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(area);

        render_header(frame, app, chunks[0], &t);
        render_context(frame, app, chunks[1], &t);
        render_editor(frame, app, chunks[2], &t);
        render_mode_indicator(frame, app, chunks[3], &t);
        keybind_bar::render(frame, chunks[4], hint, &t);
    } else {
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
        keybind_bar::render(frame, chunks[3], hint, &t);
    }
}

fn render_context(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let file_path = app.compose_file_path.as_deref().unwrap_or("");
    let line_num = app.compose_line.map(|l| l.to_string()).unwrap_or_else(|| "?".to_string());
    let ext = file_path.rsplit('.').next();

    let title = format!(" Context: {}:{} ", file_path, line_num);
    let block = Block::default()
        .title(title.as_str())
        .title_style(Style::default().fg(t.suggestion))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.suggestion))
        .style(Style::default().bg(t.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let context_len = app.compose_context.len();
    let lines: Vec<Line> = app.compose_context.iter().enumerate().map(|(i, raw)| {
        // Highlight the selected line (middle of context)
        let is_target = i == context_len / 2;

        let diff_char = raw.chars().next().unwrap_or(' ');
        let code = if raw.len() > 1 { &raw[1..] } else { "" };

        let (diff_color, bg) = match diff_char {
            '+' => (t.diff_add, Some(Color::Rgb(20, 48, 20))),
            '-' => (t.diff_remove, Some(Color::Rgb(48, 20, 20))),
            _ => (t.diff_context, None),
        };

        let effective_bg = if is_target { Some(Color::Rgb(40, 40, 0)) } else { bg };

        let mut spans = vec![Span::styled(
            diff_char.to_string(),
            Style::default().fg(diff_color),
        )];
        spans.extend(syntax::highlight(code, ext, effective_bg));
        Line::from(spans)
    }).collect();

    frame.render_widget(Paragraph::new(lines).style(Style::default().bg(t.background)), inner);
}

fn render_header(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
    let location = match (&app.compose_file_path, app.compose_line) {
        (Some(f), Some(l)) => format!(" — {}:{}", f, l),
        (Some(f), None) => format!(" — {}", f),
        _ => String::new(),
    };
    let mode_label = if app.compose_quick_mode {
        " Quick Comment"
    } else {
        " Compose Comment"
    };
    let title = format!("{} — PR #{pr_num}{location} ", mode_label);
    let block = Block::default()
        .title(title.as_str())
        .title_style(Style::default().fg(if app.compose_quick_mode { t.warning } else { t.title }).add_modifier(Modifier::BOLD))
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
