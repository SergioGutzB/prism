use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table};

use crate::app::App;
use crate::ui::components::syntax;
use crate::ui::theme::Theme;

/// Colorize a single diff line with syntax highlighting. Called only for visible lines.
fn colorize_line(raw: &str, ext: Option<&str>, t: &Theme) -> Line<'static> {
    // Header lines: no syntax
    if raw.starts_with("@@") {
        return Line::from(Span::styled(raw.to_string(), Style::default().fg(t.diff_hunk)));
    }
    if raw.starts_with("diff ") || raw.starts_with("index ")
        || raw.starts_with("---") || raw.starts_with("+++")
    {
        return Line::from(Span::styled(raw.to_string(), Style::default().fg(t.title)));
    }
    // Added line: green fg prefix + syntax highlighted code with subtle green bg
    if raw.starts_with('+') {
        let code = &raw[1..];
        let bg = Color::Rgb(20, 48, 20);
        let mut spans = vec![Span::styled("+".to_string(), Style::default().fg(t.diff_add).bg(bg))];
        spans.extend(syntax::highlight(code, ext, Some(bg)));
        return Line::from(spans);
    }
    // Removed line: red fg prefix + syntax highlighted code with subtle red bg
    if raw.starts_with('-') {
        let code = &raw[1..];
        let bg = Color::Rgb(48, 20, 20);
        let mut spans = vec![Span::styled("-".to_string(), Style::default().fg(t.diff_remove).bg(bg))];
        spans.extend(syntax::highlight(code, ext, Some(bg)));
        return Line::from(spans);
    }
    // Context line: syntax highlighted code, no bg
    if raw.starts_with(' ') {
        let code = &raw[1..];
        let mut spans = vec![Span::raw(" ")];
        spans.extend(syntax::highlight(code, ext, None));
        return Line::from(spans);
    }
    // Fallback
    Line::from(Span::styled(raw.to_string(), Style::default().fg(t.diff_context)))
}

/// Render the diff panel using the pre-split line cache.
/// Only colorizes the lines that fit in the visible area — O(height), not O(total).
pub fn render(frame: &mut Frame, app: &App, area: Rect, t: &Theme, focused: bool) {
    let border_color = if focused { t.border_focused } else { t.border };

    let title = match app.selected_pane {
        1 => " Diff [focused] ",
        _ => " Diff ",
    };

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(t.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Loading state
    if app.diff_loading || (app.current_diff.is_none() && app.pr_loading) {
        frame.render_widget(
            Paragraph::new(format!("{} Loading diff…", app.spinner_char()))
                .style(Style::default().fg(t.loading)),
            inner,
        );
        return;
    }

    let lines = match &app.diff_lines_cache {
        Some(l) => l,
        None => {
            frame.render_widget(
                Paragraph::new("No diff available.")
                    .style(Style::default().fg(t.muted)),
                inner,
            );
            return;
        }
    };

    let total = lines.len();
    if total == 0 {
        frame.render_widget(
            Paragraph::new("(empty diff)").style(Style::default().fg(t.muted)),
            inner,
        );
        return;
    }

    // Clamp scroll so we never go past the last line
    let max_scroll = total.saturating_sub(inner.height as usize);
    let scroll = app.diff_scroll.min(max_scroll);

    // Only colorize the visible slice — O(height) instead of O(total)
    let visible: Vec<Line> = lines
        .iter()
        .skip(scroll)
        .zip(
            app.diff_line_ext
                .iter()
                .skip(scroll)
                .chain(std::iter::repeat(&None)),
        )
        .take(inner.height as usize)
        .map(|(raw, ext)| colorize_line(raw, ext.as_deref(), t))
        .collect();

    frame.render_widget(
        Paragraph::new(visible).style(Style::default().bg(t.background)),
        inner,
    );

    // Vertical scrollbar (renders over the right border)
    if total > inner.height as usize {
        let mut sb_state = ScrollbarState::new(max_scroll).position(scroll);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"))
                .thumb_symbol("█")
                .track_symbol(Some("│")),
            area,
            &mut sb_state,
        );

        // Line counter in the bottom-right corner (inside the border)
        let indicator = format!(" {}/{} ", scroll + 1, total);
        let iw = indicator.len() as u16;
        let ind_area = Rect {
            x: area.right().saturating_sub(iw + 2),
            y: area.bottom().saturating_sub(1),
            width: iw,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(indicator)
                .style(Style::default().fg(t.muted).bg(t.background)),
            ind_area,
        );
    }
}

// ─── Split diff rendering ──────────────────────────────────────────────────

/// A row in a parsed side-by-side diff.
#[derive(Debug, Clone)]
enum SplitLine {
    /// Full-width line: file header, index, or @@ hunk marker.
    Wide { text: String, is_hunk: bool },
    /// Side-by-side pair: old (left) and new (right).
    Side {
        old_num: Option<u32>,
        old_text: String,
        /// True if this was a `-` line (shown with red bg on the left).
        old_removed: bool,
        new_num: Option<u32>,
        new_text: String,
        /// True if this was a `+` line (shown with green bg on the right).
        new_added: bool,
        ext: Option<String>,
    },
}

/// Parse the `@@ -old,len +new,len @@` header and return (old_start, new_start).
fn parse_hunk_header(raw: &str) -> Option<(u32, u32)> {
    let after = raw.strip_prefix("@@ -")?;
    let (old_part, rest) = after.split_once(' ')?;
    let old_start: u32 = old_part.split(',').next()?.parse().ok()?;
    let after_plus = rest.strip_prefix('+')?;
    let new_part = after_plus.split_whitespace().next()?;
    let new_start: u32 = new_part.split(',').next()?.parse().ok()?;
    Some((old_start, new_start))
}

/// Flush accumulated `-` and `+` buffers as paired `SplitLine::Side` rows.
fn flush_bufs(
    old_buf: &mut Vec<(u32, String)>,
    new_buf: &mut Vec<(u32, String)>,
    result: &mut Vec<SplitLine>,
    ext: Option<String>,
) {
    let old_len = old_buf.len();
    let new_len = new_buf.len();
    let n = old_len.max(new_len);
    for i in 0..n {
        let old_part = if i < old_len { Some(old_buf[i].clone()) } else { None };
        let new_part = if i < new_len { Some(new_buf[i].clone()) } else { None };
        result.push(SplitLine::Side {
            old_num: old_part.as_ref().map(|(n, _)| *n),
            old_text: old_part.map(|(_, t)| t).unwrap_or_default(),
            old_removed: i < old_len,
            new_num: new_part.as_ref().map(|(n, _)| *n),
            new_text: new_part.map(|(_, t)| t).unwrap_or_default(),
            new_added: i < new_len,
            ext: ext.clone(),
        });
    }
    old_buf.clear();
    new_buf.clear();
}

/// Convert a flat unified-diff line cache into paired side-by-side rows.
fn parse_to_split(lines: &[String]) -> Vec<SplitLine> {
    let mut result: Vec<SplitLine> = Vec::new();
    let mut old_buf: Vec<(u32, String)> = Vec::new();
    let mut new_buf: Vec<(u32, String)> = Vec::new();
    let mut old_n: u32 = 1;
    let mut new_n: u32 = 1;
    let mut cur_ext: Option<String> = None;

    for raw in lines {
        if raw.starts_with("diff ") || raw.starts_with("index ") {
            flush_bufs(&mut old_buf, &mut new_buf, &mut result, cur_ext.clone());
            result.push(SplitLine::Wide { text: raw.clone(), is_hunk: false });
        } else if raw.starts_with("--- ") || raw.starts_with("+++ ") {
            flush_bufs(&mut old_buf, &mut new_buf, &mut result, cur_ext.clone());
            if raw.starts_with("+++ ") {
                let path = raw
                    .trim_start_matches("+++ b/")
                    .trim_start_matches("+++ a/")
                    .trim_start_matches("+++ ");
                cur_ext = std::path::Path::new(path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|s| s.to_string());
            }
            result.push(SplitLine::Wide { text: raw.clone(), is_hunk: false });
        } else if raw.starts_with("@@") {
            flush_bufs(&mut old_buf, &mut new_buf, &mut result, cur_ext.clone());
            if let Some((o, n)) = parse_hunk_header(raw) {
                old_n = o;
                new_n = n;
            }
            result.push(SplitLine::Wide { text: raw.clone(), is_hunk: true });
        } else if raw.starts_with('-') {
            old_buf.push((old_n, raw[1..].to_string()));
            old_n += 1;
        } else if raw.starts_with('+') {
            new_buf.push((new_n, raw[1..].to_string()));
            new_n += 1;
        } else if raw.starts_with(' ') {
            flush_bufs(&mut old_buf, &mut new_buf, &mut result, cur_ext.clone());
            result.push(SplitLine::Side {
                old_num: Some(old_n),
                old_text: raw[1..].to_string(),
                old_removed: false,
                new_num: Some(new_n),
                new_text: raw[1..].to_string(),
                new_added: false,
                ext: cur_ext.clone(),
            });
            old_n += 1;
            new_n += 1;
        } else {
            flush_bufs(&mut old_buf, &mut new_buf, &mut result, cur_ext.clone());
            result.push(SplitLine::Wide { text: raw.clone(), is_hunk: false });
        }
    }
    flush_bufs(&mut old_buf, &mut new_buf, &mut result, cur_ext);
    result
}

/// Build a ratatui `Row` from a `SplitLine`.
/// Columns: old_linenum | old_content | divider | new_linenum | new_content
fn split_line_to_row<'a>(sl: &SplitLine, t: &Theme) -> Row<'a> {
    let divider = Cell::from("│").style(Style::default().fg(Color::DarkGray));

    match sl {
        SplitLine::Wide { text, is_hunk } => {
            let style = if *is_hunk {
                Style::default().fg(t.diff_hunk)
            } else {
                Style::default().fg(t.title)
            };
            Row::new(vec![
                Cell::from(""),
                Cell::from(text.clone()).style(style),
                divider,
                Cell::from(""),
                Cell::from(""),
            ])
        }
        SplitLine::Side {
            old_num, old_text, old_removed,
            new_num, new_text, new_added,
            ext,
        } => {
            let ext_ref = ext.as_deref();
            let old_bg = if *old_removed { Some(Color::Rgb(48, 20, 20)) } else { None };
            let new_bg = if *new_added { Some(Color::Rgb(20, 48, 20)) } else { None };

            let old_num_str = old_num.map(|n| n.to_string()).unwrap_or_default();
            let new_num_str = new_num.map(|n| n.to_string()).unwrap_or_default();

            // Left content cell
            let old_content: Line<'static> = if old_text.is_empty() && !old_removed {
                Line::from("")
            } else {
                let (prefix, prefix_style) = if *old_removed {
                    let bg = old_bg.unwrap_or(Color::Reset);
                    ("-", Style::default().fg(t.diff_remove).bg(bg))
                } else {
                    (" ", Style::default())
                };
                let mut spans = vec![Span::styled(prefix.to_string(), prefix_style)];
                spans.extend(syntax::highlight(old_text, ext_ref, old_bg));
                Line::from(spans)
            };

            // Right content cell
            let new_content: Line<'static> = if new_text.is_empty() && !new_added {
                Line::from("")
            } else {
                let (prefix, prefix_style) = if *new_added {
                    let bg = new_bg.unwrap_or(Color::Reset);
                    ("+", Style::default().fg(t.diff_add).bg(bg))
                } else {
                    (" ", Style::default())
                };
                let mut spans = vec![Span::styled(prefix.to_string(), prefix_style)];
                spans.extend(syntax::highlight(new_text, ext_ref, new_bg));
                Line::from(spans)
            };

            let old_num_style = Style::default()
                .fg(Color::DarkGray)
                .bg(old_bg.unwrap_or(t.background));
            let new_num_style = Style::default()
                .fg(Color::DarkGray)
                .bg(new_bg.unwrap_or(t.background));

            Row::new(vec![
                Cell::from(old_num_str).style(old_num_style),
                Cell::from(old_content),
                divider,
                Cell::from(new_num_str).style(new_num_style),
                Cell::from(new_content),
            ])
        }
    }
}

/// Render the diff as a side-by-side split view (fullscreen only).
pub fn render_split(frame: &mut Frame, app: &App, area: Rect, t: &Theme, focused: bool) {
    let border_color = if focused { t.border_focused } else { t.border };

    let block = Block::default()
        .title(" Split Diff  [Z] Unified ")
        .title_style(Style::default().fg(t.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.diff_loading || (app.current_diff.is_none() && app.pr_loading) {
        frame.render_widget(
            Paragraph::new(format!("{} Loading diff…", app.spinner_char()))
                .style(Style::default().fg(t.loading)),
            inner,
        );
        return;
    }

    let raw_lines = match &app.diff_lines_cache {
        Some(l) => l,
        None => {
            frame.render_widget(
                Paragraph::new("No diff available.").style(Style::default().fg(t.muted)),
                inner,
            );
            return;
        }
    };

    if raw_lines.is_empty() {
        frame.render_widget(
            Paragraph::new("(empty diff)").style(Style::default().fg(t.muted)),
            inner,
        );
        return;
    }

    let split_rows = parse_to_split(raw_lines);
    let total = split_rows.len();
    let height = inner.height as usize;
    let max_scroll = total.saturating_sub(height);
    let scroll = app.diff_scroll.min(max_scroll);

    // Column widths: linenum(5) | old_content | divider(1) | linenum(5) | new_content
    // Subtract 11 for the two line-num cols (5+5) and divider (1)
    let content_width = inner.width.saturating_sub(11);
    let half = content_width / 2;
    let widths = [
        Constraint::Length(5),
        Constraint::Length(half),
        Constraint::Length(1),
        Constraint::Length(5),
        Constraint::Min(1),
    ];

    let rows: Vec<Row> = split_rows
        .iter()
        .skip(scroll)
        .take(height)
        .map(|sl| split_line_to_row(sl, t))
        .collect();

    let table = Table::new(rows, widths)
        .column_spacing(0)
        .style(Style::default().bg(t.background));

    frame.render_widget(table, inner);

    // Scrollbar
    if total > height {
        let mut sb_state = ScrollbarState::new(max_scroll).position(scroll);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"))
                .thumb_symbol("█")
                .track_symbol(Some("│")),
            area,
            &mut sb_state,
        );

        let indicator = format!(" {}/{} ", scroll + 1, total);
        let iw = indicator.len() as u16;
        let ind_area = Rect {
            x: area.right().saturating_sub(iw + 2),
            y: area.bottom().saturating_sub(1),
            width: iw,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(indicator).style(Style::default().fg(t.muted).bg(t.background)),
            ind_area,
        );
    }
}

/// Render a side-by-side split diff from an explicit slice of unified-diff lines.
/// Used by screens (e.g. FileTree) that manage their own line slices.
pub fn render_split_lines(
    frame: &mut Frame,
    area: Rect,
    lines: &[String],
    scroll: usize,
    title: &str,
    spinner: Option<char>,
    t: &Theme,
    focused: bool,
) {
    let border_color = if focused { t.border_focused } else { t.border };

    let block = Block::default()
        .title(format!(" {} — Split  [Z] Unified ", title))
        .title_style(Style::default().fg(t.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(ch) = spinner {
        frame.render_widget(
            Paragraph::new(format!("{} Loading diff…", ch))
                .style(Style::default().fg(t.loading)),
            inner,
        );
        return;
    }

    if lines.is_empty() {
        frame.render_widget(
            Paragraph::new("No diff available.").style(Style::default().fg(t.muted)),
            inner,
        );
        return;
    }

    let split_rows = parse_to_split(lines);
    let total = split_rows.len();
    let height = inner.height as usize;
    let max_scroll = total.saturating_sub(height);
    let scroll = scroll.min(max_scroll);

    let content_width = inner.width.saturating_sub(11);
    let half = content_width / 2;
    let widths = [
        Constraint::Length(5),
        Constraint::Length(half),
        Constraint::Length(1),
        Constraint::Length(5),
        Constraint::Min(1),
    ];

    let rows: Vec<Row> = split_rows
        .iter()
        .skip(scroll)
        .take(height)
        .map(|sl| split_line_to_row(sl, t))
        .collect();

    let table = Table::new(rows, widths)
        .column_spacing(0)
        .style(Style::default().bg(t.background));

    frame.render_widget(table, inner);

    if total > height {
        let mut sb_state = ScrollbarState::new(max_scroll).position(scroll);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"))
                .thumb_symbol("█")
                .track_symbol(Some("│")),
            area,
            &mut sb_state,
        );
        let indicator = format!(" {}/{} ", scroll + 1, total);
        let iw = indicator.len() as u16;
        let ind_area = Rect {
            x: area.right().saturating_sub(iw + 2),
            y: area.bottom().saturating_sub(1),
            width: iw,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(indicator).style(Style::default().fg(t.muted).bg(t.background)),
            ind_area,
        );
    }
}

/// Also expose a unified (inline) render from explicit lines for the FileTree screen.
pub fn render_unified_lines(
    frame: &mut Frame,
    area: Rect,
    lines: &[String],
    scroll: usize,
    title: &str,
    spinner: Option<char>,
    t: &Theme,
    focused: bool,
) {
    let border_color = if focused { t.border_focused } else { t.border };

    let block = Block::default()
        .title(format!(" {} ", title))
        .title_style(Style::default().fg(t.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.background));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(ch) = spinner {
        frame.render_widget(
            Paragraph::new(format!("{} Loading diff…", ch))
                .style(Style::default().fg(t.loading)),
            inner,
        );
        return;
    }

    if lines.is_empty() {
        frame.render_widget(
            Paragraph::new("No diff available.").style(Style::default().fg(t.muted)),
            inner,
        );
        return;
    }

    let total = lines.len();
    let max_scroll = total.saturating_sub(inner.height as usize);
    let scroll = scroll.min(max_scroll);

    // Extract extension from the +++ line
    let ext: Option<&str> = lines.iter()
        .find(|l| l.starts_with("+++ "))
        .and_then(|l| {
            let path = l
                .trim_start_matches("+++ b/")
                .trim_start_matches("+++ a/")
                .trim_start_matches("+++ ");
            std::path::Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
        });

    let visible: Vec<Line> = lines
        .iter()
        .skip(scroll)
        .take(inner.height as usize)
        .map(|raw| colorize_line(raw, ext, t))
        .collect();

    frame.render_widget(
        Paragraph::new(visible).style(Style::default().bg(t.background)),
        inner,
    );

    if total > inner.height as usize {
        let mut sb_state = ScrollbarState::new(max_scroll).position(scroll);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"))
                .thumb_symbol("█")
                .track_symbol(Some("│")),
            area,
            &mut sb_state,
        );
        let indicator = format!(" {}/{} ", scroll + 1, total);
        let iw = indicator.len() as u16;
        let ind_area = Rect {
            x: area.right().saturating_sub(iw + 2),
            y: area.bottom().saturating_sub(1),
            width: iw,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(indicator).style(Style::default().fg(t.muted).bg(t.background)),
            ind_area,
        );
    }
}

/// Legacy function kept for compatibility — no longer called each frame.
/// Use the diff_lines_cache in App instead.
pub fn parse_diff_lines(diff: &str, _theme: &str) -> Vec<String> {
    diff.lines().map(str::to_string).collect()
}
