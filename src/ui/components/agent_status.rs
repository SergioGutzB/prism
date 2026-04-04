use std::borrow::Cow;

use ratatui::prelude::*;

use crate::agents::models::AgentStatus;
use crate::ui::theme::Theme;

/// Return a colored status line for a single agent.
pub fn status_line<'a>(
    name: &'a str,
    icon: &'a str,
    status: Option<&'a AgentStatus>,
    spinner: char,
    t: &Theme,
) -> Line<'a> {
    let (status_icon, style): (Cow<'static, str>, Style) = match status {
        None | Some(AgentStatus::Pending) => (Cow::Borrowed("○"), Style::default().fg(t.muted)),
        Some(AgentStatus::Disabled) => (Cow::Borrowed("─"), Style::default().fg(t.agent_disabled)),
        Some(AgentStatus::Running { .. }) => {
            (Cow::Owned(spinner.to_string()), Style::default().fg(t.agent_running))
        }
        Some(AgentStatus::Done { .. }) => (Cow::Borrowed("✓"), Style::default().fg(t.agent_done)),
        Some(AgentStatus::Failed { .. }) => (Cow::Borrowed("✗"), Style::default().fg(t.agent_failed)),
        Some(AgentStatus::Skipped { .. }) => (Cow::Borrowed("⊘"), Style::default().fg(t.agent_skipped)),
    };

    let detail = match status {
        Some(AgentStatus::Done { comments, elapsed_ms, input_tokens, output_tokens }) => {
            format!("{} comment(s) in {}ms  ~{}in/{}out tok",
                comments.len(), elapsed_ms, input_tokens, output_tokens)
        }
        Some(AgentStatus::Failed { error }) => error.clone(),
        Some(AgentStatus::Skipped { reason }) => reason.clone(),
        _ => String::new(),
    };

    Line::from(vec![
        Span::styled(format!(" {} ", status_icon), style),
        Span::raw(format!("{} ", icon)),
        Span::styled(name, Style::default().fg(t.foreground)),
        Span::styled(
            if detail.is_empty() { String::new() } else { format!(" — {}", detail) },
            Style::default().fg(t.muted),
        ),
    ])
}
