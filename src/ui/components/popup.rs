use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::{App, PopupKind};
use crate::ui::theme::Theme;

/// Render the current popup (if any) centered over the screen.
pub fn render_popup(frame: &mut Frame, app: &App) {
    let popup = match &app.popup {
        Some(p) => p,
        None => return,
    };

    let t = Theme::current(&app.config.ui.theme);
    let area = frame.area();
    let popup_area = centered_rect(60, 30, area);

    // Clear the area beneath the popup
    frame.render_widget(Clear, popup_area);

    let border_color = match popup.kind {
        PopupKind::Error => t.critical,
        PopupKind::Confirm => t.warning,
        PopupKind::Info => t.border_focused,
    };

    let popup_title = format!(" {} ", popup.title);
    let block = Block::default()
        .title(popup_title.as_str())
        .title_style(
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.background));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let hint = match popup.kind {
        PopupKind::Confirm => "\n\n[Enter] Confirm  [Esc] Cancel",
        _ => "\n\n[Esc] Close",
    };

    let text = format!("{}{}", popup.message, hint);
    let para = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(t.foreground).bg(t.background));

    frame.render_widget(para, inner);
}

/// Return a centered rectangle of `percent_x` width and `percent_y` height
/// within `r`.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_width = r.width * percent_x / 100;
    let popup_height = r.height * percent_y / 100;
    let x = r.x + (r.width.saturating_sub(popup_width)) / 2;
    let y = r.y + (r.height.saturating_sub(popup_height)) / 2;
    Rect {
        x,
        y,
        width: popup_width.max(1),
        height: popup_height.max(1),
    }
}
