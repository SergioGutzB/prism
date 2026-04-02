use tui_textarea::{TextArea, CursorMove};
use crossterm::event::{KeyEvent, KeyCode};

/// Reusable Vim-style editor for large text inputs.
pub struct PrismEditor<'a> {
    pub textarea: TextArea<'a>,
    pub is_insert_mode: bool,
}

impl<'a> PrismEditor<'a> {
    pub fn new(content: String) -> Self {
        let mut textarea = TextArea::new(content.lines().map(String::from).collect());
        // Default style for the editor
        textarea.set_cursor_line_style(ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::UNDERLINED));
        
        Self {
            textarea,
            is_insert_mode: false,
        }
    }

    /// Process key events with Vim emulation logic.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.is_insert_mode {
            match key.code {
                KeyCode::Esc => {
                    self.is_insert_mode = false;
                    true
                }
                _ => {
                    self.textarea.input(key);
                    true
                }
            }
        } else {
            // Normal Mode (Vim-lite)
            match key.code {
                KeyCode::Char('i') => {
                    self.is_insert_mode = true;
                    true
                }
                KeyCode::Char('j') | KeyCode::Down => { self.textarea.move_cursor(CursorMove::Down); true }
                KeyCode::Char('k') | KeyCode::Up => { self.textarea.move_cursor(CursorMove::Up); true }
                KeyCode::Char('h') | KeyCode::Left => { self.textarea.move_cursor(CursorMove::Back); true }
                KeyCode::Char('l') | KeyCode::Right => { self.textarea.move_cursor(CursorMove::Forward); true }
                KeyCode::Char('w') => { self.textarea.move_cursor(CursorMove::WordForward); true }
                KeyCode::Char('b') => { self.textarea.move_cursor(CursorMove::WordBack); true }
                KeyCode::Char('0') | KeyCode::Home => { self.textarea.move_cursor(CursorMove::Head); true }
                KeyCode::Char('$') | KeyCode::End => { self.textarea.move_cursor(CursorMove::End); true }
                KeyCode::Char('x') => { self.textarea.delete_char(); true }
                KeyCode::Char('o') => { 
                    self.textarea.move_cursor(CursorMove::End);
                    self.textarea.insert_newline();
                    self.is_insert_mode = true;
                    true
                }
                _ => false,
            }
        }
    }

    pub fn get_text(&self) -> String {
        self.textarea.lines().join("\n")
    }
}

