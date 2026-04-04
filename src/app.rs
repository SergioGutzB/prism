use std::collections::HashMap;
use tokio::sync::mpsc;
use crate::agents::models::{AgentDefinition, AgentStatus};
use crate::agents::orchestrator::AgentUpdate;
use crate::config::AppConfig;
use crate::github::models::{PrDetails, PrSummary};
use crate::review::models::ReviewDraft;
use crate::tickets::models::Ticket;
use crate::tui::keybindings::{InputMode, KeySequenceDetector};
use crate::ui::editor::PrismEditor;

/// Which screen the TUI is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    Setup,
    PrList,
    PrDetail,
    FileTree,
    ReviewCompose,
    AgentRunner,
    DoubleCheck,
    SummaryPreview,
    AgentConfig,
    AgentWizard,
    Settings,
    ClaudeCodeOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentWizardField {
    Id,
    Name,
    Icon,
    SystemPrompt,
}

impl Default for Screen {
    fn default() -> Self { Self::PrList }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupField { Owner, Repo }

#[derive(Debug, Clone)]
pub struct PopupState {
    pub title: String,
    pub message: String,
    pub kind: PopupKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupKind {
    Info, Error, Confirm, ConfirmQuit, ConfirmPublish, ConfirmRestart, ConfirmCancelAgents,
    ConfirmDeleteComment,
}

#[derive(Debug, Clone)]
pub enum PendingPublish {
    QuickComment { text: String },
    FullReview,
    RestartReview,
    RunMissingAgents,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixTaskStatus { Pending, Running, Done, Failed(String) }

#[derive(Debug, Clone)]
pub struct FixTask {
    pub index: usize,
    pub location: String,
    pub source: String,
    pub summary: String,
    pub status: FixTaskStatus,
    pub output: String,
    pub prompt: String,
}

/// Per-day usage bucket (stored as "YYYY-MM-DD" key in ModelStats.daily).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct DayStats {
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Historical token statistics per model.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct ModelStats {
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// When this model was first used (set on first call, never changed).
    #[serde(default)]
    pub start_date: Option<chrono::DateTime<chrono::Utc>>,
    /// Per-day stats for range filtering. Key: "YYYY-MM-DD".
    #[serde(default)]
    pub daily: std::collections::HashMap<String, DayStats>,
}

pub struct App {
    pub screen: Screen,
    pub config: AppConfig,
    pub agents: Vec<AgentDefinition>,
    pub pr_list: Vec<PrSummary>,
    pub pr_list_selected: usize,
    pub pr_list_loading: bool,
    pub pr_list_filter: String,
    pub current_pr: Option<PrDetails>,
    pub current_diff: Option<String>,
    pub diff_lines_cache: Option<Vec<String>>,
    pub current_ticket: Option<Ticket>,
    pub pr_loading: bool,
    pub diff_loading: bool,
    pub description_scroll: usize,
    pub draft: Option<ReviewDraft>,
    pub agent_statuses: HashMap<String, AgentStatus>,
    pub agent_rx: Option<mpsc::Receiver<AgentUpdate>>,
    pub agent_abort: Option<tokio::task::AbortHandle>,
    pub agent_filter: Option<u8>,
    pub input_mode: InputMode,
    pub key_detector: KeySequenceDetector,
    pub selected_pane: usize,
    pub diff_scroll: usize,
    pub diff_cursor: usize,
    pub diff_viewport_height: usize,
    pub github_user: Option<String>,
    pub should_quit: bool,
    pub status_message: Option<String>,
    pub popup: Option<PopupState>,

    // Editors
    pub compose_editor: PrismEditor<'static>,
    pub wizard_prompt_editor: PrismEditor<'static>,
    
    pub wizard_id: String,
    pub wizard_name: String,
    pub wizard_icon: String,
    pub wizard_field: AgentWizardField,

    pub double_check_selected: usize,
    pub double_check_pane: u8,
    pub double_check_detail_scroll: usize,
    pub summary_event_idx: usize,
    pub summary_pane: usize,
    pub summary_body_scroll: usize,
    pub summary_comments_scroll: usize,
    pub agent_config_selected: usize,
    pub screen_stack: Vec<Screen>,
    pub diff_fullscreen: bool,
    pub diff_split_mode: bool,
    pub tick: u64,
    pub file_tree_scroll: usize,
    pub file_tree_pane: u8,
    pub file_tree_line: usize,
    pub file_tree_fullscreen: bool,
    pub file_tree_split: bool,
    pub compose_quick_mode: bool,
    pub pending_publish: Option<PendingPublish>,
    pub compose_file_path: Option<String>,
    pub compose_line: Option<u32>,
    pub compose_context: Vec<String>,
    pub diff_line_ext: Vec<Option<String>>,
    /// Set to true once agent comments have been committed to the draft, preventing re-insertion.
    pub agents_committed: bool,
    pub show_help: bool,
    pub show_stats: bool,

    // Statistics
    pub stats_range: u8,   // 0 = last 7d, 1 = last 15d, 2 = last 30d, 3 = all time
    pub model_stats: HashMap<String, ModelStats>,
    
    /// Local uuid of comment pending deletion confirmation.
    pub pending_delete_comment: Option<uuid::Uuid>,
    /// Local uuid of comment being edited in ReviewCompose.
    pub editing_comment_id: Option<uuid::Uuid>,
    pub fix_tasks: Vec<FixTask>,
    pub fix_task_selected: usize,
    pub claude_output_scroll: usize,
    pub claude_output_loading: bool,
    pub setup_gh_token: String,
    pub setup_owner: String,
    pub setup_repo: String,
    pub setup_field: SetupField,
    pub setup_saving: bool,
    /// CONTRIBUTING.md or PR template fetched from the current repo (if available).
    pub project_conventions: Option<String>,
    /// Pre-rendered markdown lines for the current PR description. Populated once
    /// on PrLoaded/ConfigReloaded so render_description never re-parses per frame.
    pub pr_description_md_cache: Option<Vec<ratatui::text::Line<'static>>>,
    /// Pre-computed side-by-side split diff rows. Populated once on DiffLoaded
    /// so render_split never re-parses the full diff on every frame.
    pub split_diff_cache: Option<Vec<crate::ui::components::diff_view::SplitLine>>,
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
            agent_abort: None,
            agent_filter: None,
            input_mode: InputMode::Normal,
            key_detector: KeySequenceDetector::new(),
            selected_pane: 0,
            diff_scroll: 0,
            diff_cursor: 0,
            diff_viewport_height: 20,
            github_user: None,
            should_quit: false,
            status_message: None,
            popup: None,
            compose_editor: PrismEditor::new(String::new()),
            wizard_prompt_editor: PrismEditor::new(String::new()),
            wizard_id: String::new(),
            wizard_name: String::new(),
            wizard_icon: "🤖".to_string(),
            wizard_field: AgentWizardField::Id,
            double_check_selected: 0,
            double_check_pane: 0,
            double_check_detail_scroll: 0,
            summary_event_idx: 0,
            summary_pane: 0,
            summary_body_scroll: 0,
            summary_comments_scroll: 0,
            agent_config_selected: 0,
            screen_stack: Vec::new(),
            diff_fullscreen: false,
            diff_split_mode: false,
            tick: 0,
            file_tree_scroll: 0,
            file_tree_pane: 0,
            file_tree_line: 0,
            file_tree_fullscreen: false,
            file_tree_split: false,
            compose_quick_mode: false,
            pending_publish: None,
            compose_file_path: None,
            compose_line: None,
            compose_context: Vec::new(),
            diff_line_ext: Vec::new(),
            agents_committed: false,
            show_help: false,
            show_stats: false,
            stats_range: 3,
            model_stats: HashMap::new(),
            pending_delete_comment: None,
            editing_comment_id: None,
            fix_tasks: Vec::new(),
            fix_task_selected: 0,
            claude_output_scroll: 0,
            claude_output_loading: false,
            setup_gh_token: String::new(),
            setup_owner: String::new(),
            setup_repo: String::new(),
            setup_field: SetupField::Owner,
            setup_saving: false,
            project_conventions: None,
            pr_description_md_cache: None,
            split_diff_cache: None,
        }
    }

    pub fn navigate_to(&mut self, next: Screen) {
        let current = self.screen.clone();
        self.screen_stack.push(current);
        self.screen = next;
        self.key_detector.reset();
        self.selected_pane = 0;
    }

    pub fn navigate_back(&mut self) {
        if let Some(prev) = self.screen_stack.pop() {
            self.screen = prev;
            self.key_detector.reset();
        }
    }

    pub fn show_error(&mut self, msg: impl Into<String>) {
        self.popup = Some(PopupState { title: "Error".to_string(), message: msg.into(), kind: PopupKind::Error });
    }

    pub fn show_info(&mut self, title: impl Into<String>, msg: impl Into<String>) {
        self.popup = Some(PopupState { title: title.into(), message: msg.into(), kind: PopupKind::Info });
    }

    pub fn dismiss_popup(&mut self) { self.popup = None; }
    pub fn set_status(&mut self, msg: impl Into<String>) { self.status_message = Some(msg.into()); }
    pub fn clear_status(&mut self) { self.status_message = None; }

    pub fn filtered_prs(&self) -> Vec<&PrSummary> {
        if self.pr_list_filter.is_empty() {
            self.pr_list.iter().collect()
        } else {
            let q = self.pr_list_filter.to_lowercase();
            self.pr_list.iter().filter(|pr| pr.title.to_lowercase().contains(&q) || pr.author.to_lowercase().contains(&q) || pr.number.to_string().contains(&q)).collect()
        }
    }

    pub fn selected_pr(&self) -> Option<&PrSummary> {
        let filtered = self.filtered_prs();
        filtered.get(self.pr_list_selected).copied()
    }

    pub fn nav_down(&mut self) {
        let len = self.filtered_prs().len();
        if len > 0 && self.pr_list_selected < len - 1 { self.pr_list_selected += 1; }
    }

    pub fn nav_up(&mut self) { if self.pr_list_selected > 0 { self.pr_list_selected -= 1; } }
    pub fn go_top(&mut self) { self.pr_list_selected = 0; }
    pub fn go_bottom(&mut self) {
        let len = self.filtered_prs().len();
        if len > 0 { self.pr_list_selected = len - 1; }
    }

    pub fn spinner_char(&self) -> char {
        const FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        FRAMES[(self.tick as usize / 3) % FRAMES.len()]
    }
}
