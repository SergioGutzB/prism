use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table, TableState};

use crate::app::App;
use crate::tui::keybindings::InputMode;
use crate::ui::components::keybind_bar;
use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, app: &App) {
    let t = Theme::current(&app.config.ui.theme);
    let area = frame.area();

    // Background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.background)),
        area,
    );

    // Layout: header | body | keybind bar
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(3),
    ])
    .split(area);

    render_header(frame, app, chunks[0], &t);
    render_body(frame, app, chunks[1], &t);
    keybind_bar::render(
        frame,
        chunks[2],
        &[
            ("[jk/↑↓]", "Nav"),
            ("[Enter]", "Open"),
            ("[o]", "Browser"),
            ("[F5]", "Refresh"),
            ("[/]", "Search"),
            ("[a]", "Agents"),
            ("[S]", "Settings"),
            ("[q]", "Quit"),
            ("[?]", "Help"),
            ("[T]", "Stats"),
        ],
        &t,
    );
}

fn render_header(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let owner = &app.config.github.owner;
    let repo = &app.config.github.repo;
    let title = format!(" PRISM — {owner}/{repo} ");

    let configured = app.config.is_github_configured();
    let status = if !configured {
        Span::styled(
            " ⚠ GitHub not configured ",
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        )
    } else if app.pr_list_loading {
        Span::styled(
            format!(" {} Loading… ", app.spinner_char()),
            Style::default().fg(t.loading),
        )
    } else {
        let n = app.filtered_prs().len();
        Span::styled(
            format!(" {n} open PRs "),
            Style::default().fg(t.muted),
        )
    };

    let header_block = Block::default()
        .title(title.as_str())
        .title_alignment(Alignment::Left)
        .title_style(Style::default().fg(t.title).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border_focused))
        .style(Style::default().bg(t.background));

    let inner = header_block.inner(area);
    frame.render_widget(header_block, area);
    frame.render_widget(
        Paragraph::new(status).alignment(Alignment::Right),
        inner,
    );

    // Show search filter when searching (Insert mode) or when filter has text
    let is_searching = app.input_mode == InputMode::Insert;
    if is_searching || !app.pr_list_filter.is_empty() {
        let cursor = if is_searching { "█" } else { "" };
        let filter_text = format!("/ {}{}", app.pr_list_filter, cursor);
        let filter_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width.min(50),
            height: 1,
        };
        frame.render_widget(Clear, filter_area);
        frame.render_widget(
            Paragraph::new(filter_text)
                .style(Style::default().fg(t.highlight).add_modifier(Modifier::BOLD)),
            filter_area,
        );
    }
}

fn render_body(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    if !app.config.is_github_configured() {
        let msg = Paragraph::new(
            "\n  GitHub is not configured.\n\n\
             Please set the following environment variables:\n\
             • GITHUB_TOKEN\n\
             • GITHUB_OWNER\n\
             • GITHUB_REPO\n\n\
             Then restart PRISM.",
        )
        .style(Style::default().fg(t.warning))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.border)),
        );
        frame.render_widget(msg, area);
        return;
    }

    if app.pr_list_loading {
        let spinner = app.spinner_char();
        let msg = Paragraph::new(format!("\n  {spinner} Fetching pull requests…"))
            .style(Style::default().fg(t.loading))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.border)),
            );
        frame.render_widget(msg, area);
        return;
    }

    let prs = app.filtered_prs();

    if prs.is_empty() {
        let msg = if app.pr_list_filter.is_empty() {
            "  No open PRs found.".to_string()
        } else {
            format!("  No PRs matching \"{}\".", app.pr_list_filter)
        };
        let para = Paragraph::new(format!("\n{msg}"))
            .style(Style::default().fg(t.muted))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.border)),
            );
        frame.render_widget(para, area);
        return;
    }

    let header_cells = ["  #", "Title", "Author", "Age", "Files", "±", "Labels"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(t.title).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(0);

    let rows: Vec<Row> = prs
        .iter()
        .enumerate()
        .map(|(i, pr)| {
            let selected = i == app.pr_list_selected;
            let age = format_age(pr.updated_at);
            // PrSummary doesn't carry labels; leave blank
            let labels: String = String::new();
            let draft_marker = if pr.draft { " [draft]" } else { "" };
            let plus_minus = format!("+{} -{}", pr.additions, pr.deletions);
            let row_style = if selected {
                Style::default().bg(t.selected_bg).fg(t.selected_fg)
            } else {
                Style::default().bg(t.background).fg(t.foreground)
            };
            let indicator = if selected { "▶" } else { " " };
            Row::new(vec![
                Cell::from(format!("{}{}", indicator, pr.number)),
                Cell::from(format!("{}{}", pr.title.as_str(), draft_marker)),
                Cell::from(format!("@{}", pr.author)),
                Cell::from(age),
                Cell::from(pr.changed_files.to_string()),
                Cell::from(plus_minus),
                Cell::from(labels),
            ])
            .style(row_style)
            .height(1)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(6),
            Constraint::Min(24),
            Constraint::Length(14),
            Constraint::Length(8),
            Constraint::Length(6),
            Constraint::Length(12),
            Constraint::Min(12),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border)),
    )
    .row_highlight_style(Style::default().bg(t.selected_bg).fg(t.selected_fg));

    let mut state = TableState::default();
    state.select(Some(app.pr_list_selected));
    frame.render_stateful_widget(table, area, &mut state);

    // Vertical scrollbar
    let total = prs.len();
    let visible_height = area.height.saturating_sub(3) as usize; // header row + borders
    if total > visible_height {
        let max_s = total.saturating_sub(visible_height);
        let pos = app.pr_list_selected.min(max_s);
        let mut sb_state = ScrollbarState::new(max_s).position(pos);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"))
                .thumb_symbol("█")
                .track_symbol(Some("│")),
            area,
            &mut sb_state,
        );
    }
}

fn format_age(dt: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let diff = now.signed_duration_since(dt);
    if diff.num_days() > 0 {
        format!("{}d", diff.num_days())
    } else if diff.num_hours() > 0 {
        format!("{}h", diff.num_hours())
    } else {
        format!("{}m", diff.num_minutes().max(0))
    }
}
