/// Minimal markdown-to-ratatui renderer.
/// Handles: headers, bullets, blockquotes, fenced code blocks, and inline `**bold**` / `` `code` ``.
use ratatui::prelude::*;

use crate::ui::theme::Theme;

pub fn parse(text: &str, t: &Theme) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_code_block = false;

    for raw in text.lines() {
        // Fenced code block fence
        if raw.starts_with("```") {
            in_code_block = !in_code_block;
            if in_code_block {
                let lang = raw.trim_start_matches('`').trim();
                if !lang.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!("[{}]", lang),
                        Style::default().fg(t.muted),
                    )));
                }
            }
            continue;
        }

        if in_code_block {
            lines.push(Line::from(Span::styled(
                raw.to_string(),
                Style::default().fg(t.suggestion),
            )));
            continue;
        }

        let line = if raw.starts_with("### ") {
            Line::from(Span::styled(
                raw[4..].to_string(),
                Style::default().fg(t.foreground).add_modifier(Modifier::BOLD),
            ))
        } else if raw.starts_with("## ") {
            Line::from(Span::styled(
                raw[3..].to_string(),
                Style::default().fg(t.title).add_modifier(Modifier::BOLD),
            ))
        } else if raw.starts_with("# ") {
            Line::from(Span::styled(
                raw[2..].to_string(),
                Style::default()
                    .fg(t.title)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ))
        } else if raw.starts_with("- ") || raw.starts_with("* ") {
            let rest = parse_inline(&raw[2..], t);
            let mut spans = vec![Span::styled("  • ".to_string(), Style::default().fg(t.muted))];
            spans.extend(rest);
            Line::from(spans)
        } else if raw.starts_with("> ") {
            Line::from(Span::styled(
                format!("│ {}", &raw[2..]),
                Style::default().fg(t.muted),
            ))
        } else if raw.is_empty() {
            Line::from("")
        } else {
            Line::from(parse_inline(raw, t))
        };

        lines.push(line);
    }

    lines
}

/// Parse inline markdown (`` `code` `` and `**bold**`) into styled spans.
fn parse_inline(text: &str, t: &Theme) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        // Check **bold** first (before single *)
        if let Some(start) = remaining.find("**") {
            if start > 0 {
                spans.push(Span::styled(
                    remaining[..start].to_string(),
                    Style::default().fg(t.foreground),
                ));
            }
            remaining = &remaining[start + 2..];
            if let Some(end) = remaining.find("**") {
                spans.push(Span::styled(
                    remaining[..end].to_string(),
                    Style::default().fg(t.foreground).add_modifier(Modifier::BOLD),
                ));
                remaining = &remaining[end + 2..];
            } else {
                spans.push(Span::styled(
                    format!("**{}", remaining),
                    Style::default().fg(t.foreground),
                ));
                break;
            }
        }
        // Check `code`
        else if let Some(start) = remaining.find('`') {
            if start > 0 {
                spans.push(Span::styled(
                    remaining[..start].to_string(),
                    Style::default().fg(t.foreground),
                ));
            }
            remaining = &remaining[start + 1..];
            if let Some(end) = remaining.find('`') {
                spans.push(Span::styled(
                    remaining[..end].to_string(),
                    Style::default().fg(t.suggestion),
                ));
                remaining = &remaining[end + 1..];
            } else {
                spans.push(Span::styled(
                    format!("`{}", remaining),
                    Style::default().fg(t.foreground),
                ));
                break;
            }
        } else {
            spans.push(Span::styled(
                remaining.to_string(),
                Style::default().fg(t.foreground),
            ));
            break;
        }
    }

    if spans.is_empty() {
        spans.push(Span::raw(""));
    }
    spans
}
