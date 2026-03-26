#![allow(dead_code, unused_imports, unused_variables)]

mod agents;
mod app;
mod config;
mod error;
mod github;
mod review;
mod tickets;
mod tui;
mod ui;

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::app::{App, Screen};
use crate::tui::event::{spawn_event_reader, AppEvent};
use crate::tui::keybindings::{map_key, Action, InputMode, KeySequence};

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env if present
    let _ = dotenvy::dotenv();

    // Init tracing to a file (not stdout — stdout is the TUI)
    let _guard = init_tracing();

    info!("prism v{}", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let config = match config::AppConfig::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load configuration: {e}");
            std::process::exit(1);
        }
    };

    // Load agent definitions
    let agents = match agents::loader::load_agents(&config) {
        Ok(a) => {
            info!("Loaded {} agents", a.len());
            a
        }
        Err(e) => {
            eprintln!("Warning: failed to load agents: {e}");
            vec![]
        }
    };

    // Build initial app state
    let mut app = App::new(config.clone(), agents);

    // Setup terminal
    let mut terminal = tui::terminal::init()?;

    // Event channel
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();

    // Spawn crossterm reader
    spawn_event_reader(event_tx.clone());

    // If GitHub is configured, kick off PR list loading in background
    if config.is_github_configured() {
        let tx = event_tx.clone();
        let cfg = config.clone();
        tokio::spawn(async move {
            match load_pr_list(&cfg).await {
                Ok(prs) => {
                    let _ = tx.send(AppEvent::PrListLoaded(prs));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("Failed to load PRs: {e}")));
                }
            }
        });
    } else {
        app.pr_list_loading = false;
    }

    // ── Main event loop ────────────────────────────────────────────────────
    loop {
        // Drain any pending agent updates from the mpsc receiver
        // We must collect first to avoid borrow conflict
        let agent_updates: Vec<_> = if let Some(rx) = &mut app.agent_rx {
            let mut updates = Vec::new();
            while let Ok(update) = rx.try_recv() {
                updates.push(update);
            }
            updates
        } else {
            Vec::new()
        };
        for update in agent_updates {
            handle_agent_update(&mut app, update);
        }

        // Render
        terminal.draw(|frame| {
            ui::render(frame, &app);
        })?;

        // Wait for the next event
        let event = match event_rx.recv().await {
            Some(e) => e,
            None => break,
        };

        match event {
            AppEvent::Tick => {
                app.tick = app.tick.wrapping_add(1);
            }

            AppEvent::Key(key_event) => {
                // Feed the key sequence detector first (only in Normal mode)
                if app.input_mode == InputMode::Normal {
                    if let Some(seq) = app.key_detector.feed(&key_event) {
                        handle_sequence(&mut app, seq);
                        continue;
                    }
                }

                // Map key → action
                if let Some(action) = map_key(&key_event, &app.input_mode) {
                    handle_action(&mut app, action, &event_tx, &config).await;
                }

                if app.should_quit {
                    break;
                }
            }

            AppEvent::PrListLoaded(prs) => {
                info!("PR list loaded: {} PRs", prs.len());
                app.pr_list = prs;
                app.pr_list_loading = false;
                if app.pr_list.is_empty() {
                    app.set_status("No open PRs found.");
                }
            }

            AppEvent::PrLoaded(pr) => {
                info!("PR #{} loaded", pr.number);
                app.current_pr = Some(*pr);
                app.pr_loading = false;
            }

            AppEvent::DiffLoaded(diff) => {
                info!("Diff loaded ({} chars)", diff.len());
                app.current_diff = Some(diff);
            }

            AppEvent::TicketLoaded(ticket) => {
                info!("Ticket loaded: {}", ticket.is_some());
                app.current_ticket = ticket;
            }

            AppEvent::AgentUpdate(update) => {
                handle_agent_update(&mut app, update);
            }

            AppEvent::Error(msg) => {
                app.pr_list_loading = false;
                app.pr_loading = false;
                app.show_error(msg);
            }
        }
    }

    // Restore terminal
    tui::terminal::restore(&mut terminal)?;
    Ok(())
}

// ── Event handlers ─────────────────────────────────────────────────────────

fn handle_sequence(app: &mut App, seq: KeySequence) {
    match seq {
        KeySequence::GoTop => {
            app.pr_list_selected = 0;
            app.diff_scroll = 0;
        }
        KeySequence::Delete => {
            // Delete current item in DoubleCheck (reject it)
            if app.screen == Screen::DoubleCheck {
                if let Some(draft) = &mut app.draft {
                    if let Some(comment) = draft.comments.get_mut(app.double_check_selected) {
                        comment.status = crate::review::models::CommentStatus::Rejected;
                    }
                }
            }
        }
        KeySequence::ColonQuit => {
            app.should_quit = true;
        }
        KeySequence::ColonWrite => {
            // Save / confirm depending on screen
            app.set_status("Saved.");
        }
    }
}

async fn handle_action(
    app: &mut App,
    action: Action,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    config: &config::AppConfig,
) {
    // If a popup is open, only allow closing it
    if app.popup.is_some() {
        match action {
            Action::Quit | Action::Back | Action::Confirm | Action::ExitInsert => {
                app.dismiss_popup();
            }
            _ => {}
        }
        return;
    }

    match action {
        Action::Quit => {
            app.should_quit = true;
        }

        Action::Back => {
            if app.input_mode == InputMode::Insert {
                app.input_mode = InputMode::Normal;
                return;
            }
            app.navigate_back();
        }

        Action::ExitInsert => {
            app.input_mode = InputMode::Normal;
        }

        Action::EnterInsert => {
            if matches!(app.screen, Screen::ReviewCompose) {
                app.input_mode = InputMode::Insert;
            }
        }

        Action::NavDown => {
            match app.screen {
                Screen::PrList => app.nav_down(),
                Screen::DoubleCheck => {
                    let len = app.draft.as_ref().map(|d| d.comments.len()).unwrap_or(0);
                    if app.double_check_selected + 1 < len {
                        app.double_check_selected += 1;
                    }
                }
                Screen::AgentConfig => {
                    if app.agent_config_selected + 1 < app.agents.len() {
                        app.agent_config_selected += 1;
                    }
                }
                Screen::SummaryPreview => {
                    if app.summary_event_idx + 1 < 3 {
                        app.summary_event_idx += 1;
                    }
                }
                Screen::PrDetail => {
                    app.diff_scroll = app.diff_scroll.saturating_add(3);
                }
                _ => {}
            }
        }

        Action::NavUp => {
            match app.screen {
                Screen::PrList => app.nav_up(),
                Screen::DoubleCheck => {
                    if app.double_check_selected > 0 {
                        app.double_check_selected -= 1;
                    }
                }
                Screen::AgentConfig => {
                    if app.agent_config_selected > 0 {
                        app.agent_config_selected -= 1;
                    }
                }
                Screen::SummaryPreview => {
                    if app.summary_event_idx > 0 {
                        app.summary_event_idx -= 1;
                    }
                }
                Screen::PrDetail => {
                    app.diff_scroll = app.diff_scroll.saturating_sub(3);
                }
                _ => {}
            }
        }

        Action::GoTop => {
            app.pr_list_selected = 0;
            app.diff_scroll = 0;
        }

        Action::GoBottom => {
            app.go_bottom();
        }

        Action::ScrollDown => {
            app.diff_scroll = app.diff_scroll.saturating_add(10);
        }

        Action::ScrollUp => {
            app.diff_scroll = app.diff_scroll.saturating_sub(10);
        }

        Action::PageDown => {
            app.diff_scroll = app.diff_scroll.saturating_add(25);
        }

        Action::PageUp => {
            app.diff_scroll = app.diff_scroll.saturating_sub(25);
        }

        Action::NextPane => {
            app.selected_pane = (app.selected_pane + 1) % 3;
        }

        Action::PrevPane => {
            app.selected_pane = if app.selected_pane == 0 { 2 } else { app.selected_pane - 1 };
        }

        Action::Confirm => match app.screen {
            Screen::PrList => {
                open_pr(app, event_tx, config).await;
            }
            Screen::ReviewCompose => {
                if app.input_mode == InputMode::Insert {
                    // Insert a newline
                    let pos = app.compose_cursor.min(app.compose_text.len());
                    app.compose_text.insert(pos, '\n');
                    app.compose_cursor += 1;
                } else {
                    // Save comment and navigate to DoubleCheck
                    save_compose_comment(app);
                    app.navigate_to(Screen::DoubleCheck);
                }
            }
            Screen::SummaryPreview => {
                app.set_status("Publishing… (not yet implemented)");
            }
            Screen::DoubleCheck => {
                // Toggle approve/reject
                toggle_comment(app);
            }
            _ => {}
        },

        Action::GenerateReview => {
            if app.screen == Screen::PrDetail {
                if !config.is_llm_configured() {
                    app.show_info(
                        "AI Unavailable",
                        "Claude CLI not found and no API key configured.\nInstall Claude Code or set ANTHROPIC_API_KEY.",
                    );
                } else {
                    start_agent_runner(app, config);
                }
            }
        }

        Action::HybridReview => {
            if app.screen == Screen::PrDetail {
                if !config.is_llm_configured() {
                    app.show_info(
                        "AI Unavailable",
                        "Claude CLI not found and no API key configured.\nInstall Claude Code or set ANTHROPIC_API_KEY.",
                    );
                } else {
                    start_agent_runner(app, config);
                }
            }
        }

        Action::ManualComment => {
            if app.screen == Screen::PrDetail {
                app.compose_text.clear();
                app.compose_cursor = 0;
                app.navigate_to(Screen::ReviewCompose);
            }
        }

        Action::FileTree => {
            if matches!(app.screen, Screen::PrDetail | Screen::ReviewCompose) {
                app.navigate_to(Screen::FileTree);
            }
        }

        Action::AgentConfig => {
            app.navigate_to(Screen::AgentConfig);
        }

        Action::Settings => {
            app.navigate_to(Screen::Settings);
        }

        Action::OpenBrowser => {
            let url = match &app.screen {
                Screen::PrList => app.selected_pr().map(|p| p.html_url.clone()),
                Screen::PrDetail => app.current_pr.as_ref().map(|p| p.html_url.clone()),
                _ => None,
            };
            if let Some(url) = url {
                // Best-effort: open URL in the default browser
                let _ = std::process::Command::new("xdg-open")
                    .arg(&url)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
            }
        }

        Action::Refresh => {
            if app.screen == Screen::PrList && config.is_github_configured() {
                app.pr_list_loading = true;
                app.pr_list.clear();
                let tx = event_tx.clone();
                let cfg = config.clone();
                tokio::spawn(async move {
                    match load_pr_list(&cfg).await {
                        Ok(prs) => {
                            let _ = tx.send(AppEvent::PrListLoaded(prs));
                        }
                        Err(e) => {
                            let _ = tx.send(AppEvent::Error(format!("Refresh failed: {e}")));
                        }
                    }
                });
            }
        }

        Action::Search => {
            if app.screen == Screen::PrList {
                app.input_mode = InputMode::Insert;
                app.pr_list_filter.clear();
            }
        }

        Action::ToggleItem => {
            match app.screen {
                Screen::DoubleCheck => toggle_comment(app),
                Screen::AgentConfig => {
                    if let Some(agent) = app.agents.get_mut(app.agent_config_selected) {
                        agent.agent.enabled = !agent.agent.enabled;
                    }
                }
                _ => {}
            }
        }

        Action::SelectAll => {
            if app.screen == Screen::DoubleCheck {
                if let Some(draft) = &mut app.draft {
                    draft.approve_all();
                }
            }
        }

        Action::DeselectAll => {
            if app.screen == Screen::DoubleCheck {
                if let Some(draft) = &mut app.draft {
                    for c in &mut draft.comments {
                        c.status = crate::review::models::CommentStatus::Rejected;
                    }
                }
            }
        }

        Action::PreviewSummary => {
            if app.screen == Screen::DoubleCheck {
                app.navigate_to(Screen::SummaryPreview);
            }
        }

        Action::CheckFile => {
            if app.screen == Screen::FileTree {
                if let Some(draft) = &mut app.draft {
                    let key = draft
                        .file_checklist
                        .keys()
                        .nth(app.pr_list_selected)
                        .cloned();
                    if let Some(k) = key {
                        let val = draft.file_checklist.get_mut(&k).unwrap();
                        *val = !*val;
                    }
                }
            }
        }

        Action::Delete => {
            if app.input_mode == InputMode::Insert {
                // Backspace in editor
                match app.screen {
                    Screen::ReviewCompose => {
                        if app.compose_cursor > 0 {
                            let pos = (app.compose_cursor - 1).min(app.compose_text.len());
                            if app.compose_text.is_char_boundary(pos) {
                                app.compose_text.remove(pos);
                                app.compose_cursor = pos;
                            }
                        }
                    }
                    Screen::PrList => {
                        app.pr_list_filter.pop();
                    }
                    _ => {}
                }
            }
        }

        Action::Char(c) => {
            if app.input_mode == InputMode::Insert {
                match app.screen {
                    Screen::ReviewCompose => {
                        let pos = app.compose_cursor.min(app.compose_text.len());
                        app.compose_text.insert(pos, c);
                        app.compose_cursor += c.len_utf8();
                    }
                    Screen::PrList => {
                        app.pr_list_filter.push(c);
                        app.pr_list_selected = 0;
                    }
                    _ => {}
                }
            }
        }

        Action::FilterAgent(n) => {
            if app.screen == Screen::DoubleCheck {
                if app.agent_filter == Some(n) {
                    app.agent_filter = None; // toggle off
                } else {
                    app.agent_filter = Some(n);
                }
                app.double_check_selected = 0;
            }
        }

        Action::NavLeft | Action::NavRight | Action::Publish => {
            // Not yet implemented
        }
    }
}

fn handle_agent_update(app: &mut App, update: agents::orchestrator::AgentUpdate) {
    use agents::models::AgentStatus;

    // If all agents are done/failed/skipped and we're on AgentRunner, move to DoubleCheck
    app.agent_statuses.insert(update.agent_id, update.status);

    if app.screen == Screen::AgentRunner {
        let all_done = app.agents.iter().all(|a| {
            match app.agent_statuses.get(&a.agent.id) {
                Some(AgentStatus::Done { .. })
                | Some(AgentStatus::Failed { .. })
                | Some(AgentStatus::Skipped { .. })
                | Some(AgentStatus::Disabled) => true,
                _ => false,
            }
        });

        if all_done && !app.agents.is_empty() {
            // Collect comments from Done agents into the draft
            if let Some(draft) = &mut app.draft {
                for (id, status) in &app.agent_statuses {
                    if let AgentStatus::Done { comments, .. } = status {
                        for c in comments {
                            // Only add if not already present (avoid double-add on re-render)
                            if !draft.comments.iter().any(|existing| existing.id == c.id) {
                                draft.comments.push(c.clone());
                            }
                        }
                    }
                    let _ = id;
                }
            }
            app.navigate_to(Screen::DoubleCheck);
        }
    }
}

// ── Helper functions ───────────────────────────────────────────────────────

async fn open_pr(
    app: &mut App,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    config: &config::AppConfig,
) {
    let pr_num = match app.selected_pr() {
        Some(pr) => pr.number,
        None => return,
    };

    app.current_pr = None;
    app.current_diff = None;
    app.current_ticket = None;
    app.pr_loading = true;
    app.diff_scroll = 0;
    app.selected_pane = 1;

    app.navigate_to(Screen::PrDetail);

    // Load PR details
    {
        let tx = event_tx.clone();
        let cfg = config.clone();
        tokio::spawn(async move {
            match load_pr_details(&cfg, pr_num).await {
                Ok(pr) => {
                    let _ = tx.send(AppEvent::PrLoaded(Box::new(pr)));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("Failed to load PR: {e}")));
                }
            }
        });
    }

    // Load diff
    {
        let tx = event_tx.clone();
        let cfg = config.clone();
        tokio::spawn(async move {
            match load_pr_diff(&cfg, pr_num).await {
                Ok(diff) => {
                    let _ = tx.send(AppEvent::DiffLoaded(diff));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("Failed to load diff: {e}")));
                }
            }
        });
    }

    // Load ticket (best-effort)
    {
        let tx = event_tx.clone();
        let cfg = config.clone();
        tokio::spawn(async move {
            // Try to extract ticket ref from PR title / branch — stub for now
            let ticket = load_ticket_for_pr(&cfg, pr_num).await.ok().flatten();
            let _ = tx.send(AppEvent::TicketLoaded(ticket));
        });
    }
}

fn start_agent_runner(app: &mut App, config: &config::AppConfig) {
    use agents::context::ReviewContext;
    use agents::orchestrator::Orchestrator;
    use review::models::{ReviewDraft, ReviewMode};

    let pr = match &app.current_pr {
        Some(pr) => pr.clone(),
        None => {
            app.show_error("No PR loaded — open a PR first.");
            return;
        }
    };

    let diff = app.current_diff.clone().unwrap_or_default();
    let ticket = app.current_ticket.clone();
    let ctx = ReviewContext::from_pr(&pr, &diff, ticket);

    let pr_num = pr.number;
    app.draft = Some(ReviewDraft::new(pr_num, ReviewMode::AiOnly));
    app.agent_statuses.clear();

    // Pre-populate Pending status so the screen renders immediately
    for agent in app.agents.iter().filter(|a| a.agent.enabled) {
        app.agent_statuses.insert(
            agent.agent.id.clone(),
            agents::models::AgentStatus::Pending,
        );
    }

    app.navigate_to(Screen::AgentRunner);

    // Launch the orchestrator; updates arrive via app.agent_rx
    let orchestrator = Orchestrator::new(config.clone());
    let rx = orchestrator.run_all(app.agents.clone(), ctx);
    app.agent_rx = Some(rx);
}

fn save_compose_comment(app: &mut App) {
    use review::models::{CommentSource, GeneratedComment, ReviewDraft, ReviewMode, Severity};
    let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
    if app.draft.is_none() {
        app.draft = Some(ReviewDraft::new(pr_num, ReviewMode::ManualOnly));
    }
    if !app.compose_text.trim().is_empty() {
        let comment = GeneratedComment::new(
            CommentSource::Manual,
            app.compose_text.clone(),
            Severity::Suggestion,
            None,
            None,
        );
        if let Some(draft) = &mut app.draft {
            draft.add_comment(comment);
        }
    }
    app.compose_text.clear();
    app.compose_cursor = 0;
}

fn toggle_comment(app: &mut App) {
    use review::models::CommentStatus;
    if let Some(draft) = &mut app.draft {
        if let Some(comment) = draft.comments.get_mut(app.double_check_selected) {
            comment.status = match comment.status {
                CommentStatus::Pending | CommentStatus::Rejected => CommentStatus::Approved,
                CommentStatus::Approved => CommentStatus::Rejected,
            };
        }
    }
}

// ── GitHub API calls ───────────────────────────────────────────────────────

fn make_github_api(config: &config::AppConfig) -> anyhow::Result<github::api::GitHubApi> {
    let client = github::client::GitHubClient::new(
        &config.github.token,
        &config.github.owner,
        &config.github.repo,
    )?;
    Ok(github::api::GitHubApi::new(client))
}

async fn load_pr_list(config: &config::AppConfig) -> anyhow::Result<Vec<github::models::PrSummary>> {
    let api = make_github_api(config)?;
    api.list_prs(config.github.per_page).await
}

async fn load_pr_details(
    config: &config::AppConfig,
    pr_num: u64,
) -> anyhow::Result<github::models::PrDetails> {
    let api = make_github_api(config)?;
    api.get_pr_details(pr_num).await
}

async fn load_pr_diff(
    config: &config::AppConfig,
    pr_num: u64,
) -> anyhow::Result<String> {
    let api = make_github_api(config)?;
    api.get_pr_diff(pr_num).await
}

async fn load_ticket_for_pr(
    _config: &config::AppConfig,
    _pr_num: u64,
) -> anyhow::Result<Option<tickets::models::Ticket>> {
    // Ticket loading is best-effort and provider-dependent.
    // Returning None as a safe default for now.
    Ok(None)
}

// ── Tracing init ───────────────────────────────────────────────────────────

fn init_tracing() -> Option<()> {
    // Write logs to a file if PRISM_LOG_FILE is set, otherwise discard
    if let Ok(log_file) = std::env::var("PRISM_LOG_FILE") {
        if let Ok(file) = std::fs::File::create(&log_file) {
            tracing_subscriber::fmt()
                .with_writer(std::sync::Mutex::new(file))
                .with_env_filter(
                    EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| EnvFilter::new("prism=info")),
                )
                .init();
            return Some(());
        }
    }
    // No log output when running TUI (avoids corrupting the screen)
    Some(())
}
