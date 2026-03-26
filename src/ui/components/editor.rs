use ratatui::prelude::*;
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::ui::theme::Theme;

/// A simple multi-line text editor state.
#[derive(Debug, Default, Clone)]
pub struct EditorState {
    pub text: String,
    pub cursor: usize,
}

impl EditorState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a character at the cursor position.
    pub fn insert_char(&mut self, c: char) {
        let pos = self.cursor.min(self.text.len());
        self.text.insert(pos, c);
        self.cursor += c.len_utf8();
    }

    /// Insert a newline at the cursor position.
    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    /// Delete the character before the cursor (backspace).
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        // Find the previous char boundary
        let mut prev = self.cursor - 1;
        while !self.text.is_char_boundary(prev) {
            prev -= 1;
        }
        self.text.remove(prev);
        self.cursor = prev;
    }

    /// Clear all text.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// Word count.
    pub fn word_count(&self) -> usize {
        self.text.split_whitespace().count()
    }
}

/// Render the editor widget into `area`.
pub fn render(
    frame: &mut Frame,
    state: &EditorState,
    area: Rect,
    t: &Theme,
    focused: bool,
    title: &str,
) {
    let border_color = if focused { t.border_focused } else { t.border };

    let mut display = state.text.clone();
    if focused {
        let pos = state.cursor.min(display.len());
        display.insert(pos, '█');
    }

    let para = Paragraph::new(display)
        .block(
            Block::default()
                .title(title)
                .title_style(Style::default().fg(if focused { t.warning } else { t.muted }))
                .borders(ratatui::widgets::Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(t.background)),
        )
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(t.foreground).bg(t.background));

    frame.render_widget(para, area);
}
