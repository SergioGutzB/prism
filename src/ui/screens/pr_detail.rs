use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::App;
use crate::ui::components::{diff_view, keybind_bar, ticket_panel};
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

    // Body: 2 or 3 column layout depending on whether we have a ticket
    let has_ticket = app.current_ticket.is_some()
        || app.config.tickets.providers.iter().any(|p| p.enabled);

    if has_ticket {
        // 3 columns: desc (30%) | diff (45%) | ticket (25%)
        let body = Layout::horizontal([
            Constraint::Percentage(30),
            Constraint::Percentage(45),
            Constraint::Percentage(25),
        ])
        .split(chunks[1]);
        render_description(frame, app, body[0], &t);
        render_diff_panel(frame, app, body[1], &t);
        ticket_panel::render(frame, app, body[2], &t);
    } else {
        // 2 columns: desc (35%) | diff (65%)
        let body = Layout::horizontal([
            Constraint::Percentage(35),
            Constraint::Percentage(65),
        ])
        .split(chunks[1]);
        render_description(frame, app, body[0], &t);
        render_diff_panel(frame, app, body[1], &t);
    }

    let llm_hint = if app.config.is_llm_configured() {
        ("[r]", "AI Review")
    } else {
        ("[r]", "AI (unavail)")
    };

    keybind_bar::render(
        frame,
        chunks[2],
        &[
            ("[Esc]", "Back"),
            llm_hint,
            ("[c]", "Comment"),
            ("[H]", "Hybrid"),
            ("[f]", "Files"),
            ("[o]", "Browser"),
            ("[Tab]", "Pane"),
            ("[jk]", "Scroll"),
        ],
        &t,
    );
}

fn render_header(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let pr = match &app.current_pr {
        Some(pr) => pr,
        None => {
            frame.render_widget(
                Paragraph::new(" Loading PR… ").style(Style::default().fg(t.loading)),
                area,
            );
            return;
        }
    };

    let title = format!(" PR #{} — {} ", pr.number, pr.title);
    let meta = format!(
        " @{} → {} into {} | +{} -{} ",
        pr.author, pr.head_branch, pr.base_branch, pr.additions, pr.deletions
    );

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
        Paragraph::new(meta).style(Style::default().fg(t.muted)),
        inner,
    );
}

fn render_description(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let focused = app.selected_pane == 0;
    let border_color = if focused { t.border_focused } else { t.border };

    let block = Block::default()
        .title(" Description ")
        .title_style(Style::default().fg(t.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.background).fg(t.foreground));

    if app.pr_loading {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(format!("{} Loading…", app.spinner_char()))
                .style(Style::default().fg(t.loading)),
            inner,
        );
        return;
    }

    let body = app
        .current_pr
        .as_ref()
        .map(|pr| pr.body.as_str())
        .unwrap_or("No description.");

    let para = Paragraph::new(body)
        .block(block)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(t.foreground).bg(t.background));

    frame.render_widget(para, area);
}

fn render_diff_panel(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let focused = app.selected_pane == 1;
    diff_view::render(frame, app, area, t, focused);
}
