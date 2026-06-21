use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::ui::theme::Theme;

/// Width of a single binding entry: `[key] desc`
fn binding_width(key: &str, desc: &str) -> u16 {
    (key.len() + 1 + desc.len()) as u16
}

/// Separator width between two bindings on the same line.
const SEP: u16 = 2;

/// Pack bindings into lines that fit within `inner_width`, return the line groups.
fn pack_lines<'a>(bindings: &[(&'a str, &'a str)], inner_width: u16) -> Vec<Vec<(&'a str, &'a str)>> {
    let mut lines: Vec<Vec<(&str, &str)>> = Vec::new();
    let mut current: Vec<(&str, &str)> = Vec::new();
    let mut used: u16 = 0;

    for &(key, desc) in bindings {
        let w = binding_width(key, desc);
        let needed = if current.is_empty() { w } else { SEP + w };
        if !current.is_empty() && used + needed > inner_width {
            lines.push(std::mem::take(&mut current));
            used = 0;
        }
        if current.is_empty() {
            used = w;
        } else {
            used += needed;
        }
        current.push((key, desc));
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Height (including borders) needed to display all `bindings` within `total_width`.
pub fn height_for(bindings: &[(&str, &str)], total_width: u16) -> u16 {
    let inner = total_width.saturating_sub(2); // subtract border columns
    if inner == 0 || bindings.is_empty() {
        return 3;
    }
    let lines = pack_lines(bindings, inner);
    (lines.len() as u16) + 2 // +2 for top/bottom borders
}

/// Render a keybind bar that wraps to multiple lines when the terminal is too narrow.
pub fn render(frame: &mut Frame, area: Rect, bindings: &[(&str, &str)], t: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border))
        .style(Style::default().bg(t.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if bindings.is_empty() || inner.width == 0 {
        return;
    }

    let line_groups = pack_lines(bindings, inner.width);
    let text: Vec<Line> = line_groups
        .into_iter()
        .map(|group| {
            let mut spans: Vec<Span> = Vec::new();
            for (i, (key, desc)) in group.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw("  "));
                }
                spans.push(Span::styled(
                    *key,
                    Style::default()
                        .fg(t.keybind_key)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(*desc, Style::default().fg(t.keybind_desc)));
            }
            Line::from(spans)
        })
        .collect();

    frame.render_widget(Paragraph::new(text), inner);
}
