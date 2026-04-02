use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::review::models::{CommentSource, CommentStatus, GeneratedComment, Severity};
use crate::ui::theme::Theme;

/// Render a single comment card inside the given area.
pub fn render(
    frame: &mut Frame,
    comment: &GeneratedComment,
    area: Rect,
    t: &Theme,
    selected: bool,
) {
    let border_color = if selected { t.border_focused } else { t.border };

    let sev_color = match comment.severity {
        Severity::Critical => t.critical,
        Severity::Warning => t.warning,
        Severity::Suggestion => t.suggestion,
        Severity::Praise => t.praise,
    };

    let status_mark = match comment.status {
        CommentStatus::Approved => Span::styled("✓ ", Style::default().fg(t.agent_done)),
        CommentStatus::Rejected => Span::styled("✗ ", Style::default().fg(t.agent_failed)),
        CommentStatus::Pending => Span::styled("○ ", Style::default().fg(t.muted)),
    };

    let source_text = match &comment.source {
        CommentSource::Agent { agent_name, agent_icon, .. } => {
            format!("{} {}", agent_icon, agent_name)
        }
        CommentSource::Manual => "✍ Manual".to_string(),
        CommentSource::GithubReview { user, state, .. } => format!("💬 {} ({})", user, state),
    };

    let title = format!(
        " {} [{}] {} ",
        source_text,
        comment.severity,
        comment.file_path.as_deref().unwrap_or(""),
    );

    let block = Block::default()
        .title(title.as_str())
        .title_style(Style::default().fg(sev_color))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.background));

    let body = comment.effective_body();
    let text = vec![
        Line::from(vec![
            status_mark,
            Span::styled(body, Style::default().fg(t.foreground)),
        ]),
    ];

    let para = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(para, area);
}
