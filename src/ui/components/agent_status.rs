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
    let (status_icon, style) = match status {
        None | Some(AgentStatus::Pending) => ("○", Style::default().fg(t.muted)),
        Some(AgentStatus::Disabled) => ("─", Style::default().fg(t.agent_disabled)),
        Some(AgentStatus::Running { .. }) => {
            (&*Box::leak(spinner.to_string().into_boxed_str()), Style::default().fg(t.agent_running))
        }
        Some(AgentStatus::Done { .. }) => ("✓", Style::default().fg(t.agent_done)),
        Some(AgentStatus::Failed { .. }) => ("✗", Style::default().fg(t.agent_failed)),
        Some(AgentStatus::Skipped { .. }) => ("⊘", Style::default().fg(t.agent_skipped)),
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
