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
    ClaudeCodeOutput,
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
    /// Requires Enter to confirm quit, Esc to cancel.
    ConfirmQuit,
    ConfirmPublish,
    /// Confirm restarting the review (clears existing comments).
    ConfirmRestart,
}

#[derive(Debug, Clone)]
pub enum PendingPublish {
    QuickComment { text: String },
    FullReview,
    RestartReview,
    RunMissingAgents,
}

/// Status of a single AI-fix task (one per review comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixTaskStatus {
    Pending,
    Running,
    Done,
    Failed(String),
}

/// One item in the AI-fix task list — corresponds to a single review comment
/// that Claude will apply to the codebase.
#[derive(Debug, Clone)]
pub struct FixTask {
    /// 1-based display index shown in the UI.
    pub index: usize,
    /// Location string: "src/foo.rs:42" or "(general)".
    pub location: String,
    /// Source agent name or "Manual".
    pub source: String,
    /// Truncated first line of the comment body shown in the task list.
    pub summary: String,
    pub status: FixTaskStatus,
    /// Accumulated streaming output from Claude for this task.
    pub output: String,
    /// The full prompt sent to Claude — stored for per-task retry.
    pub prompt: String,
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
    /// Pre-split diff lines — rebuilt only when current_diff changes.
    /// Stored as raw strings; colorization happens in render (O(visible only)).
    pub diff_lines_cache: Option<Vec<String>>,
    pub current_ticket: Option<Ticket>,
    pub pr_loading: bool,
    /// True while the diff is being fetched (independent of pr_loading).
    pub diff_loading: bool,
    /// Scroll offset for the Description panel (pane 0).
    pub description_scroll: usize,

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
    pub diff_scroll: usize,
    pub diff_viewport_height: usize,
    pub github_user: Option<String>,
    pub should_quit: bool,
    pub status_message: Option<String>,
    pub popup: Option<PopupState>,

    // ReviewCompose / editor state
    pub compose_text: String,
    pub compose_cursor: usize,

    // DoubleCheck selection and detail panel
    pub double_check_selected: usize,
    pub double_check_pane: u8,          // 0 = list, 1 = detail
    pub double_check_detail_scroll: usize,

    // SummaryPreview state
    pub summary_event_idx: usize,
    pub summary_pane: usize,           // 0 = body, 1 = comments
    pub summary_body_scroll: usize,
    pub summary_comments_scroll: usize,

    // AgentConfig selection
    pub agent_config_selected: usize,

    // Screen history for back-navigation
    pub screen_stack: Vec<Screen>,

    // Diff view options
    /// When true, diff fills the full body area hiding description/ticket panels.
    pub diff_fullscreen: bool,

    // Spinner tick counter (incremented on Tick events)
    pub tick: u64,

    // File tree detail panel scroll
    pub file_tree_scroll: usize,

    // File tree pane state
    pub file_tree_pane: u8,            // 0 = file list, 1 = detail panel
    pub file_tree_line: usize,         // selected line in detail panel

    pub compose_quick_mode: bool,
    pub pending_publish: Option<PendingPublish>,

    // Inline comment state
    pub compose_file_path: Option<String>,  // file for inline comment
    pub compose_line: Option<u32>,          // line for inline comment
    pub compose_context: Vec<String>,       // surrounding diff lines for context

    // Per-diff-line precomputed file extension for syntax highlighting
    pub diff_line_ext: Vec<Option<String>>,

    // Overlay visibility
    pub show_help: bool,
    pub show_stats: bool,

    // Token consumption statistics
    pub token_input_total: u64,
    pub token_output_total: u64,
    pub token_calls_total: u64,

    // Claude Code AI-fix screen state
    /// Ordered list of per-comment fix tasks.
    pub fix_tasks: Vec<FixTask>,
    /// Index of the task currently visible in the output panel.
    pub fix_task_selected: usize,
    /// Scroll offset for the output panel of the selected task.
    pub claude_output_scroll: usize,
    /// True while any fix task is pending or running.
    pub claude_output_loading: bool,

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
            diff_lines_cache: None,
            current_ticket: None,
            pr_loading: false,
            diff_loading: false,
            description_scroll: 0,
            draft: None,
            agent_statuses: HashMap::new(),
            agent_rx: None,
            agent_filter: None,
            input_mode: InputMode::Normal,
            key_detector: KeySequenceDetector::new(),
            selected_pane: 0,
            diff_scroll: 0,
            diff_viewport_height: 20,
            github_user: None,
            should_quit: false,
            status_message: None,
            popup: None,
            compose_text: String::new(),
            compose_cursor: 0,
            compose_quick_mode: false,
            pending_publish: None,
            double_check_selected: 0,
            double_check_pane: 0,
            double_check_detail_scroll: 0,
            summary_event_idx: 0, // default: COMMENT
            summary_pane: 0,
            summary_body_scroll: 0,
            summary_comments_scroll: 0,
            agent_config_selected: 0,
            screen_stack: Vec::new(),
            diff_fullscreen: false,
            tick: 0,
            file_tree_scroll: 0,
            file_tree_pane: 0,
            file_tree_line: 0,
            compose_file_path: None,
            compose_line: None,
            compose_context: Vec::new(),
            diff_line_ext: Vec::new(),
            show_help: false,
            show_stats: false,
            token_input_total: 0,
            token_output_total: 0,
            token_calls_total: 0,
            fix_tasks: Vec::new(),
            fix_task_selected: 0,
            claude_output_scroll: 0,
            claude_output_loading: false,
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
        // Reset screen-specific transient state
        if matches!(self.screen, Screen::FileTree) {
            self.file_tree_scroll = 0;
            self.file_tree_pane = 0;
            self.file_tree_line = 0;
        }
        if matches!(self.screen, Screen::DoubleCheck) {
            self.double_check_pane = 0;
            self.double_check_detail_scroll = 0;
        }
        if !matches!(self.screen, Screen::PrDetail) {
            self.diff_fullscreen = false;
        }
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
