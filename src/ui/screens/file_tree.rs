use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Gauge, Row, Table, TableState};

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
        Constraint::Length(3),
    ])
    .split(area);

    render_header(frame, app, chunks[0], &t);
    render_table(frame, app, chunks[1], &t);
    render_progress(frame, app, chunks[2], &t);
    keybind_bar::render(
        frame,
        chunks[3],
        &[
            ("[Esc]", "Back"),
            ("[jk]", "Nav"),
            ("[x]", "Check"),
            ("[A]", "Check all"),
            ("[D]", "Uncheck all"),
        ],
        &t,
    );
}

fn render_header(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
    let title = format!(" File Tree — PR #{pr_num} ");
    let block = Block::default()
        .title(title.as_str())
        .title_style(Style::default().fg(t.title).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border_focused))
        .style(Style::default().bg(t.background));
    frame.render_widget(block, area);
}

fn render_table(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let draft = match &app.draft {
        Some(d) => d,
        None => {
            frame.render_widget(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.border))
                    .style(Style::default().bg(t.background).fg(t.muted)),
                area,
            );
            return;
        }
    };

    let header_cells = ["", "File", "Comments", "Reviewed"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(t.title).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1);

    let rows: Vec<Row> = draft
        .file_checklist
        .iter()
        .enumerate()
        .map(|(i, (path, checked))| {
            let selected = i == app.pr_list_selected;
            let check = if *checked { "✓" } else { " " };
            let comment_count = draft
                .comments
                .iter()
                .filter(|c| c.file_path.as_deref() == Some(path))
                .count();
            let row_style = if selected {
                Style::default().bg(t.selected_bg).fg(t.selected_fg)
            } else {
                Style::default().bg(t.background).fg(t.foreground)
            };
            Row::new(vec![
                Cell::from(check).style(Style::default().fg(t.agent_done)),
                Cell::from(path.as_str()),
                Cell::from(comment_count.to_string()),
                Cell::from(if *checked { "✓" } else { "○" }),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Min(30),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border)),
    );

    let mut state = TableState::default();
    state.select(Some(app.pr_list_selected));
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_progress(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let (total, checked) = match &app.draft {
        Some(d) => {
            let total = d.file_checklist.len();
            let checked = d.file_checklist.values().filter(|&&v| v).count();
            (total, checked)
        }
        None => (0, 0),
    };

    let ratio = if total > 0 {
        checked as f64 / total as f64
    } else {
        0.0
    };

    let label = format!(" {checked}/{total} files reviewed ");
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).border_style(
            Style::default().fg(t.border),
        ))
        .gauge_style(Style::default().fg(t.agent_done).bg(t.background))
        .label(label)
        .ratio(ratio);

    frame.render_widget(gauge, area);
}
