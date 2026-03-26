use std::collections::HashMap;

use tokio::sync::mpsc;

use crate::agents::models::{AgentDefinition, AgentStatus};
use crate::agents::orchestrator::AgentUpdate;
use crate::config::AppConfig;
use crate::github::models::{PrDetails, PrSummary};
use crate::review::models::ReviewDraft;
use crate::tickets::models::Ticket;
use crate::tui::keybindings::{InputMode, KeySequenceDetector};

/// Which screen the TUI is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    Setup,   // First-run wizard when GitHub is not configured
    PrList,
    PrDetail,
    FileTree,
    ReviewCompose,
    AgentRunner,
    DoubleCheck,
    SummaryPreview,
    AgentConfig,
    Settings,
}

impl Default for Screen {
    fn default() -> Self {
        Self::PrList
    }
}

/// Which field is focused in the Setup wizard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupField {
    Owner,
    Repo,
}

/// A popup that can overlay any screen.
#[derive(Debug, Clone)]
pub struct PopupState {
    pub title: String,
    pub message: String,
    pub kind: PopupKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupKind {
    Info,
    Error,
    Confirm,
}

/// Global application state — all mutable state lives here.
pub struct App {
    pub screen: Screen,
    pub config: AppConfig,
    pub agents: Vec<AgentDefinition>,

    // PR list state
    pub pr_list: Vec<PrSummary>,
    pub pr_list_selected: usize,
    pub pr_list_loading: bool,
    pub pr_list_filter: String,

    // Current PR state
    pub current_pr: Option<PrDetails>,
    pub current_diff: Option<String>,
    pub current_ticket: Option<Ticket>,
    pub pr_loading: bool,

    // Review draft
    pub draft: Option<ReviewDraft>,

    // Agent state
    pub agent_statuses: HashMap<String, AgentStatus>,
    pub agent_rx: Option<mpsc::Receiver<AgentUpdate>>,
    pub agent_filter: Option<u8>,

    // UI state
    pub input_mode: InputMode,
    pub key_detector: KeySequenceDetector,
    pub selected_pane: usize,
    pub diff_scroll: u16,
    pub should_quit: bool,
    pub status_message: Option<String>,
    pub popup: Option<PopupState>,

    // ReviewCompose / editor state
    pub compose_text: String,
    pub compose_cursor: usize,

    // DoubleCheck selection
    pub double_check_selected: usize,

    // SummaryPreview — which review event radio is selected
    pub summary_event_idx: usize,

    // AgentConfig selection
    pub agent_config_selected: usize,

    // Screen history for back-navigation
    pub screen_stack: Vec<Screen>,

    // Spinner tick counter (incremented on Tick events)
    pub tick: u64,

    // Setup wizard state
    pub setup_gh_token: String,       // token detected from gh CLI
    pub setup_owner: String,          // editable owner field
    pub setup_repo: String,           // editable repo field
    pub setup_field: SetupField,      // which field is focused
    pub setup_saving: bool,           // true while saving to disk
}

impl App {
    pub fn new(config: AppConfig, agents: Vec<AgentDefinition>) -> Self {
        Self {
            screen: Screen::PrList,
            config,
            agents,
            pr_list: Vec::new(),
            pr_list_selected: 0,
            pr_list_loading: true,
            pr_list_filter: String::new(),
            current_pr: None,
            current_diff: None,
            current_ticket: None,
            pr_loading: false,
            draft: None,
            agent_statuses: HashMap::new(),
            agent_rx: None,
            agent_filter: None,
            input_mode: InputMode::Normal,
            key_detector: KeySequenceDetector::new(),
            selected_pane: 0,
            diff_scroll: 0,
            should_quit: false,
            status_message: None,
            popup: None,
            compose_text: String::new(),
            compose_cursor: 0,
            double_check_selected: 0,
            summary_event_idx: 1, // default: COMMENT
            agent_config_selected: 0,
            screen_stack: Vec::new(),
            tick: 0,
            setup_gh_token: String::new(),
            setup_owner: String::new(),
            setup_repo: String::new(),
            setup_field: SetupField::Owner,
            setup_saving: false,
        }
    }

    // ── Navigation helpers ──────────────────────────────────────────────────

    /// Push the current screen to history and navigate to `next`.
    pub fn navigate_to(&mut self, next: Screen) {
        let current = self.screen.clone();
        self.screen_stack.push(current);
        self.screen = next;
        self.key_detector.reset();
        self.selected_pane = 0;
    }

    /// Navigate back to the previous screen (if any).
    pub fn navigate_back(&mut self) {
        if let Some(prev) = self.screen_stack.pop() {
            self.screen = prev;
            self.key_detector.reset();
        }
    }

    // ── Popup helpers ───────────────────────────────────────────────────────

    pub fn show_error(&mut self, msg: impl Into<String>) {
        self.popup = Some(PopupState {
            title: "Error".to_string(),
            message: msg.into(),
            kind: PopupKind::Error,
        });
    }

    pub fn show_info(&mut self, title: impl Into<String>, msg: impl Into<String>) {
        self.popup = Some(PopupState {
            title: title.into(),
            message: msg.into(),
            kind: PopupKind::Info,
        });
    }

    pub fn dismiss_popup(&mut self) {
        self.popup = None;
    }

    // ── Status bar helpers ──────────────────────────────────────────────────

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some(msg.into());
    }

    pub fn clear_status(&mut self) {
        self.status_message = None;
    }

    // ── PR list helpers ─────────────────────────────────────────────────────

    pub fn filtered_prs(&self) -> Vec<&PrSummary> {
        if self.pr_list_filter.is_empty() {
            self.pr_list.iter().collect()
        } else {
            let q = self.pr_list_filter.to_lowercase();
            self.pr_list
                .iter()
                .filter(|pr| {
                    pr.title.to_lowercase().contains(&q)
                        || pr.author.to_lowercase().contains(&q)
                        || pr.number.to_string().contains(&q)
                })
                .collect()
        }
    }

    pub fn selected_pr(&self) -> Option<&PrSummary> {
        let filtered = self.filtered_prs();
        filtered.get(self.pr_list_selected).copied()
    }

    pub fn nav_down(&mut self) {
        let len = self.filtered_prs().len();
        if len > 0 && self.pr_list_selected < len - 1 {
            self.pr_list_selected += 1;
        }
    }

    pub fn nav_up(&mut self) {
        if self.pr_list_selected > 0 {
            self.pr_list_selected -= 1;
        }
    }

    pub fn go_top(&mut self) {
        self.pr_list_selected = 0;
    }

    pub fn go_bottom(&mut self) {
        let len = self.filtered_prs().len();
        if len > 0 {
            self.pr_list_selected = len - 1;
        }
    }

    // ── Spinner helper ──────────────────────────────────────────────────────

    pub fn spinner_char(&self) -> char {
        const FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        FRAMES[(self.tick as usize / 3) % FRAMES.len()]
    }
}
