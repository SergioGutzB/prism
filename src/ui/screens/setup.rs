use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::{App, SetupField};
use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, app: &App) {
    let t = Theme::current(&app.config.ui.theme);
    let area = frame.area();

    // Dim background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.background)),
        area,
    );

    // Center a dialog box
    let dialog = centered_rect(60, 18, area);
    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .title(" PRISM — First-run Setup ")
        .title_style(
            Style::default()
                .fg(t.title)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.title))
        .style(Style::default().bg(t.background));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    let rows = Layout::vertical([
        Constraint::Length(2), // intro text
        Constraint::Length(1), // spacer
        Constraint::Length(1), // token line
        Constraint::Length(1), // spacer
        Constraint::Length(1), // owner label
        Constraint::Length(3), // owner input
        Constraint::Length(1), // repo label
        Constraint::Length(3), // repo input
        Constraint::Min(0),    // spacer
        Constraint::Length(1), // hint
    ])
    .split(inner);

    // Intro
    let intro_text = if app.setup_gh_token.is_empty() {
        "GitHub CLI (gh) not detected. Enter credentials manually."
    } else {
        "GitHub CLI detected. Confirm the repository to use:"
    };
    frame.render_widget(
        Paragraph::new(intro_text)
            .style(Style::default().fg(t.muted))
            .wrap(Wrap { trim: false }),
        rows[0],
    );

    // Token status
    let token_line = if app.setup_gh_token.is_empty() {
        Line::from(vec![
            Span::styled("Token: ", Style::default().fg(t.muted)),
            Span::styled("not found — set GITHUB_TOKEN env var", Style::default().fg(t.agent_failed)),
        ])
    } else {
        Line::from(vec![
            Span::styled("Token: ", Style::default().fg(t.muted)),
            Span::styled("✓ from gh auth", Style::default().fg(t.agent_done)),
        ])
    };
    frame.render_widget(Paragraph::new(token_line), rows[2]);

    // Owner label + input
    frame.render_widget(
        Paragraph::new("Owner / organization:").style(Style::default().fg(t.muted)),
        rows[4],
    );
    let owner_focused = app.setup_field == SetupField::Owner;
    let owner_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if owner_focused { t.border_focused } else { t.border }))
        .style(Style::default().bg(t.background));
    let owner_inner = owner_block.inner(rows[5]);
    frame.render_widget(owner_block, rows[5]);
    frame.render_widget(
        Paragraph::new(app.setup_owner.as_str()).style(Style::default().fg(t.foreground)),
        owner_inner,
    );
    if owner_focused {
        frame.set_cursor_position((
            owner_inner.x + app.setup_owner.len() as u16,
            owner_inner.y,
        ));
    }

    // Repo label + input
    frame.render_widget(
        Paragraph::new("Repository name:").style(Style::default().fg(t.muted)),
        rows[6],
    );
    let repo_focused = app.setup_field == SetupField::Repo;
    let repo_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if repo_focused { t.border_focused } else { t.border }))
        .style(Style::default().bg(t.background));
    let repo_inner = repo_block.inner(rows[7]);
    frame.render_widget(repo_block, rows[7]);
    frame.render_widget(
        Paragraph::new(app.setup_repo.as_str()).style(Style::default().fg(t.foreground)),
        repo_inner,
    );
    if repo_focused {
        frame.set_cursor_position((
            repo_inner.x + app.setup_repo.len() as u16,
            repo_inner.y,
        ));
    }

    // Bottom hint
    let hint = if app.setup_saving {
        Line::from(Span::styled(
            "Saving to ~/.config/prism/config.toml…",
            Style::default().fg(t.agent_running),
        ))
    } else {
        Line::from(vec![
            Span::styled("[Tab]", Style::default().fg(t.keybind_key).bg(t.label_bg)),
            Span::raw(" Switch field  "),
            Span::styled("[Enter]", Style::default().fg(t.keybind_key).bg(t.label_bg)),
            Span::raw(" Confirm  "),
            Span::styled("[Esc]", Style::default().fg(t.keybind_key).bg(t.label_bg)),
            Span::raw(" Quit"),
        ])
    };
    frame.render_widget(Paragraph::new(hint), rows[9]);
}

fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_width = r.width * percent_x / 100;
    let x = r.x + (r.width.saturating_sub(popup_width)) / 2;
    let y = r.y + (r.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width: popup_width.min(r.width),
        height: height.min(r.height),
    }
}
