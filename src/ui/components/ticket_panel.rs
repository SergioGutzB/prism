use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::App;
use crate::ui::theme::Theme;

/// Render the ticket info panel (right column in PrDetail).
pub fn render(frame: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let focused = app.selected_pane == 2;
    let border_color = if focused { t.border_focused } else { t.border };

    let block = Block::default()
        .title(" Ticket ")
        .title_style(Style::default().fg(t.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.background));

    match &app.current_ticket {
        None => {
            if app.pr_loading {
                let inner = block.inner(area);
                frame.render_widget(block, area);
                frame.render_widget(
                    Paragraph::new(format!("{} Loading ticket…", app.spinner_char()))
                        .style(Style::default().fg(t.loading)),
                    inner,
                );
            } else {
                let inner = block.inner(area);
                frame.render_widget(block, area);
                frame.render_widget(
                    Paragraph::new("No ticket linked\nor ticket unavailable.")
                        .style(Style::default().fg(t.muted)),
                    inner,
                );
            }
        }
        Some(ticket) => {
            let inner = block.inner(area);
            frame.render_widget(block, area);

            let priority = ticket
                .priority
                .as_deref()
                .unwrap_or("—");
            let assignee = ticket.assignee.as_deref().unwrap_or("—");
            let desc = ticket.description.as_deref().unwrap_or("No description.");

            let text = format!(
                "{} — {}\n\nStatus: {}\nType: {}\nPriority: {}\nAssignee: {}\nProvider: {}\n\n{}",
                ticket.key,
                ticket.title,
                ticket.status,
                ticket.ticket_type,
                priority,
                assignee,
                ticket.provider,
                desc,
            );

            let para = Paragraph::new(text)
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(t.foreground).bg(t.background));

            frame.render_widget(para, inner);
        }
    }
}
