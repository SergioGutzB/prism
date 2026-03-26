use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Whether the TUI is accepting vim-navigation keys or text input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Insert,
}

impl Default for InputMode {
    fn default() -> Self {
        Self::Normal
    }
}

/// Recognized multi-key sequences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySequence {
    /// `gg` — go to top
    GoTop,
    /// `dd` — delete current item
    Delete,
    /// `:q` — quit
    ColonQuit,
    /// `:w` — save/confirm
    ColonWrite,
}

const SEQUENCE_TIMEOUT: Duration = Duration::from_millis(500);

/// Tracks the last keypress to detect multi-key sequences.
#[derive(Debug, Default)]
pub struct KeySequenceDetector {
    last_key: Option<KeyCode>,
    last_at: Option<Instant>,
    /// If we just saw `:`, we're in colon-command mode
    colon_pending: bool,
}

impl KeySequenceDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a key event. Returns a completed sequence if one was detected.
    pub fn feed(&mut self, key: &KeyEvent) -> Option<KeySequence> {
        let code = key.code;
        let now = Instant::now();
        let within_timeout = self
            .last_at
            .map(|t| now.duration_since(t) < SEQUENCE_TIMEOUT)
            .unwrap_or(false);

        // Handle colon-command mode
        if self.colon_pending && within_timeout {
            self.colon_pending = false;
            self.last_key = None;
            self.last_at = None;
            return match code {
                KeyCode::Char('q') => Some(KeySequence::ColonQuit),
                KeyCode::Char('w') => Some(KeySequence::ColonWrite),
                _ => None,
            };
        }

        // Start colon-command mode
        if code == KeyCode::Char(':') {
            self.colon_pending = true;
            self.last_at = Some(now);
            self.last_key = Some(code);
            return None;
        }

        // Check for double-key sequences
        if within_timeout {
            let seq = match (self.last_key, code) {
                (Some(KeyCode::Char('g')), KeyCode::Char('g')) => Some(KeySequence::GoTop),
                (Some(KeyCode::Char('d')), KeyCode::Char('d')) => Some(KeySequence::Delete),
                _ => None,
            };
            if seq.is_some() {
                self.last_key = None;
                self.last_at = None;
                self.colon_pending = false;
                return seq;
            }
        }

        // Store for next
        self.last_key = Some(code);
        self.last_at = Some(now);
        self.colon_pending = false;
        None
    }

    /// Reset state (e.g., on screen change).
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// High-level actions mapped from key events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    Back,
    Confirm,
    NavUp,
    NavDown,
    NavLeft,
    NavRight,
    NextPane,
    PrevPane,
    GoTop,
    GoBottom,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    GenerateReview,
    ManualComment,
    HybridReview,
    Publish,
    OpenBrowser,
    Refresh,
    Search,
    AgentConfig,
    Settings,
    ToggleItem,
    SelectAll,
    DeselectAll,
    PreviewSummary,
    CheckFile,
    FileTree,
    Delete,
    EnterInsert,
    ExitInsert,
    Char(char),
    FilterAgent(u8),
}

/// Map a raw `KeyEvent` to an `Action` given the current input mode.
pub fn map_key(key: &KeyEvent, mode: &InputMode) -> Option<Action> {
    match mode {
        InputMode::Insert => match key.code {
            KeyCode::Esc => Some(Action::ExitInsert),
            KeyCode::Char(c) => Some(Action::Char(c)),
            KeyCode::Backspace => Some(Action::Delete),
            KeyCode::Enter => Some(Action::Confirm),
            KeyCode::Up => Some(Action::NavUp),
            KeyCode::Down => Some(Action::NavDown),
            KeyCode::Left => Some(Action::NavLeft),
            KeyCode::Right => Some(Action::NavRight),
            _ => None,
        },
        InputMode::Normal => {
            // Ctrl-C always quits
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                return match key.code {
                    KeyCode::Char('c') => Some(Action::Quit),
                    _ => None,
                };
            }
            match key.code {
                // Quit
                KeyCode::Char('q') => Some(Action::Quit),
                // Navigation
                KeyCode::Char('j') | KeyCode::Down => Some(Action::NavDown),
                KeyCode::Char('k') | KeyCode::Up => Some(Action::NavUp),
                KeyCode::Char('h') | KeyCode::Left => Some(Action::NavLeft),
                KeyCode::Char('l') | KeyCode::Right => Some(Action::NavRight),
                KeyCode::Char('G') => Some(Action::GoBottom),
                KeyCode::Char('J') => Some(Action::ScrollDown),
                KeyCode::Char('K') => Some(Action::ScrollUp),
                KeyCode::PageDown => Some(Action::PageDown),
                KeyCode::PageUp => Some(Action::PageUp),
                // Pane navigation
                KeyCode::Tab => Some(Action::NextPane),
                KeyCode::BackTab => Some(Action::PrevPane),
                // Confirm / back
                KeyCode::Enter => Some(Action::Confirm),
                KeyCode::Esc => Some(Action::Back),
                // Review modes
                KeyCode::Char('r') => Some(Action::GenerateReview),
                KeyCode::Char('c') => Some(Action::ManualComment),
                // 'h' is also NavLeft above — override for hybrid:
                // Use uppercase H for hybrid to avoid clash with vim left
                KeyCode::Char('H') => Some(Action::HybridReview),
                // Publish
                KeyCode::Char('p') => Some(Action::Publish),
                // Misc
                KeyCode::Char('o') => Some(Action::OpenBrowser),
                KeyCode::F(5) => Some(Action::Refresh),
                KeyCode::Char('/') => Some(Action::Search),
                KeyCode::Char('a') => Some(Action::AgentConfig),
                KeyCode::Char('S') => Some(Action::Settings),
                KeyCode::Char(' ') => Some(Action::ToggleItem),
                KeyCode::Char('A') => Some(Action::SelectAll),
                KeyCode::Char('D') => Some(Action::DeselectAll),
                KeyCode::Char('P') => Some(Action::PreviewSummary),
                KeyCode::Char('x') => Some(Action::CheckFile),
                KeyCode::Char('f') => Some(Action::FileTree),
                KeyCode::Char('i') => Some(Action::EnterInsert),
                // Filter by agent number (1-7)
                KeyCode::Char(c @ '1'..='7') => {
                    Some(Action::FilterAgent(c as u8 - b'0'))
                }
                _ => None,
            }
        }
    }
}
