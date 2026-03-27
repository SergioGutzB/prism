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

use crate::app::{App, PopupKind, PopupState, Screen};
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
        // Try to auto-detect credentials from gh CLI
        let gh_token = config::AppConfig::gh_token();
        if let Some(token) = gh_token {
            let (owner, repo) = config::AppConfig::gh_current_repo()
                .unwrap_or_default();
            app.setup_gh_token = token;
            app.setup_owner = owner;
            app.setup_repo = repo;
        }
        app.screen = Screen::Setup;
        app.pr_list_loading = false;
    }

    // ── Main event loop ────────────────────────────────────────────────────
    // Render at a fixed 16ms (~60fps) independently of incoming events.
    // Events are processed as soon as they arrive via tokio::select!.
    let mut render_ticker = tokio::time::interval(std::time::Duration::from_millis(16));
    render_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased; // prefer events over render ticks to stay responsive

            // ── Incoming async events ──────────────────────────────────────
            maybe_event = event_rx.recv() => {
                let event = match maybe_event {
                    Some(e) => e,
                    None => break,
                };
                match event {
                    AppEvent::Key(key_event) => {
                        if app.input_mode == InputMode::Normal {
                            if let Some(seq) = app.key_detector.feed(&key_event) {
                                handle_sequence(&mut app, seq);
                                if app.should_quit { break; }
                                continue;
                            }
                        }
                        if let Some(action) = map_key(&key_event, &app.input_mode) {
                            handle_action(&mut app, action, &event_tx, &config).await;
                        }
                        if app.should_quit { break; }
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
                        // Pre-split lines so render only colorizes the visible slice
                        app.diff_lines_cache = Some(
                            diff.lines().map(str::to_string).collect()
                        );
                        // Precompute file extension for each diff line (used for syntax highlighting)
                        let diff_line_ext = {
                            let mut current_ext: Option<String> = None;
                            diff.lines().map(|line| {
                                if line.starts_with("diff --git ") {
                                    current_ext = line.split(" b/").nth(1)
                                        .and_then(|p| p.rsplit('.').next())
                                        .map(|s| s.to_string());
                                }
                                current_ext.clone()
                            }).collect()
                        };
                        app.diff_line_ext = diff_line_ext;
                        app.current_diff = Some(diff);
                        app.diff_loading = false;
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
                        app.diff_loading = false;
                        app.show_error(msg);
                    }

                    AppEvent::PublishDone => {
                        app.show_info("Published", "Review submitted to GitHub successfully.");
                        app.draft = None;
                        app.screen_stack.clear();
                        app.screen = Screen::PrList;
                    }

                    AppEvent::PublishFailed(msg) => {
                        app.show_error(format!("Publish failed: {}", msg));
                    }

                    AppEvent::SetupSaved(token, owner, repo) => {
                        app.setup_saving = false;
                        app.config.github.token = token;
                        app.config.github.owner = owner.clone();
                        app.config.github.repo = repo.clone();
                        app.screen = Screen::PrList;
                        app.pr_list_loading = true;
                        let tx = event_tx.clone();
                        let cfg = app.config.clone();
                        tokio::spawn(async move {
                            match load_pr_list(&cfg).await {
                                Ok(prs) => { let _ = tx.send(AppEvent::PrListLoaded(prs)); }
                                Err(e) => { let _ = tx.send(AppEvent::Error(format!("Failed to load PRs: {e}"))); }
                            }
                        });
                    }

                    AppEvent::SetupFailed(msg) => {
                        app.setup_saving = false;
                        app.show_error(format!("Could not save config: {}", msg));
                    }
                }
            }

            // ── Render tick (60fps) ────────────────────────────────────────
            _ = render_ticker.tick() => {
                app.tick = app.tick.wrapping_add(1);

                // Drain agent updates (non-blocking)
                let agent_updates: Vec<_> = if let Some(rx) = &mut app.agent_rx {
                    let mut updates = Vec::new();
                    while let Ok(u) = rx.try_recv() { updates.push(u); }
                    updates
                } else {
                    Vec::new()
                };
                for update in agent_updates {
                    handle_agent_update(&mut app, update);
                }

                terminal.draw(|frame| ui::render(frame, &app))?;
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
            app.popup = Some(PopupState {
                title: "Quit Prism".to_string(),
                message: "Are you sure you want to quit?".to_string(),
                kind: PopupKind::ConfirmQuit,
            });
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
            Action::Confirm => {
                let is_quit = app.popup.as_ref().map(|p| p.kind == PopupKind::ConfirmQuit).unwrap_or(false);
                app.dismiss_popup();
                if is_quit {
                    app.should_quit = true;
                }
            }
            Action::Quit | Action::Back | Action::ExitInsert => {
                app.dismiss_popup();
            }
            _ => {}
        }
        return;
    }

    // Setup wizard intercepts all input
    if app.screen == Screen::Setup {
        handle_setup_action(app, action, event_tx).await;
        return;
    }

    match action {
        Action::Quit => {
            app.popup = Some(PopupState {
                title: "Quit Prism".to_string(),
                message: "Are you sure you want to quit?".to_string(),
                kind: PopupKind::ConfirmQuit,
            });
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
                Screen::FileTree => {
                    if app.file_tree_pane == 1 {
                        // Move line selection in detail panel
                        let file_count = app.draft.as_ref()
                            .and_then(|d| d.file_checklist.keys().nth(app.pr_list_selected))
                            .map(|path| count_file_diff_lines(app, path))
                            .unwrap_or(0);
                        if app.file_tree_line + 1 < file_count {
                            app.file_tree_line += 1;
                        }
                    } else {
                        let len = app.draft.as_ref().map(|d| d.file_checklist.len()).unwrap_or(0);
                        if app.pr_list_selected + 1 < len {
                            app.pr_list_selected += 1;
                            app.file_tree_scroll = 0;
                            app.file_tree_line = 0;
                        }
                    }
                }
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
                Screen::PrDetail => {
                    match app.selected_pane {
                        0 => app.description_scroll = app.description_scroll.saturating_add(3),
                        _ => {
                            let max_lines = app.diff_lines_cache.as_ref().map(|l| l.len()).unwrap_or(0);
                            app.diff_scroll = (app.diff_scroll + 3).min(max_lines);
                        }
                    }
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
                Screen::FileTree => {
                    if app.file_tree_pane == 1 {
                        if app.file_tree_line > 0 {
                            app.file_tree_line -= 1;
                        }
                    } else {
                        if app.pr_list_selected > 0 {
                            app.pr_list_selected -= 1;
                            app.file_tree_scroll = 0;
                            app.file_tree_line = 0;
                        }
                    }
                }
                Screen::AgentConfig => {
                    if app.agent_config_selected > 0 {
                        app.agent_config_selected -= 1;
                    }
                }
                Screen::PrDetail => {
                    match app.selected_pane {
                        0 => app.description_scroll = app.description_scroll.saturating_sub(3),
                        _ => app.diff_scroll = app.diff_scroll.saturating_sub(3),
                    }
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
            if app.screen == Screen::FileTree {
                app.file_tree_scroll = app.file_tree_scroll.saturating_add(5);
            } else {
                let max_lines = app.diff_lines_cache.as_ref().map(|l| l.len()).unwrap_or(0);
                app.diff_scroll = (app.diff_scroll + 10).min(max_lines);
            }
        }

        Action::ScrollUp => {
            if app.screen == Screen::FileTree {
                app.file_tree_scroll = app.file_tree_scroll.saturating_sub(5);
            } else {
                app.diff_scroll = app.diff_scroll.saturating_sub(10);
            }
        }

        Action::PageDown => {
            let max_lines = app.diff_lines_cache.as_ref().map(|l| l.len()).unwrap_or(0);
            app.diff_scroll = (app.diff_scroll + 25).min(max_lines);
        }

        Action::PageUp => {
            app.diff_scroll = app.diff_scroll.saturating_sub(25);
        }

        Action::ToggleFullscreen => {
            if app.screen == Screen::PrDetail {
                app.diff_fullscreen = !app.diff_fullscreen;
            }
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
            Screen::FileTree => {
                // Jump to this file's section in the diff view
                if let Some(draft) = &app.draft {
                    if let Some(path) = draft.file_checklist.keys().nth(app.pr_list_selected).cloned() {
                        if let Some(lines) = &app.diff_lines_cache {
                            let target = format!("diff --git a/{} b/{}", path, path);
                            if let Some(pos) = lines.iter().position(|l| l == &target) {
                                app.diff_scroll = pos;
                            }
                        }
                        app.selected_pane = 1;
                        app.navigate_back();
                    }
                }
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
                publish_review(app, event_tx, config).await;
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
                app.compose_file_path = None;
                app.compose_line = None;
                app.compose_context.clear();
                app.navigate_to(Screen::ReviewCompose);
            } else if app.screen == Screen::FileTree && app.file_tree_pane == 1 {
                // Get selected file and line for inline comment
                if let Some(path) = app.draft.as_ref()
                    .and_then(|d| d.file_checklist.keys().nth(app.pr_list_selected))
                    .cloned()
                {
                    let diff_lines = extract_file_diff_for_compose(app, &path);
                    let actual_line = get_diff_line_number(&diff_lines, app.file_tree_line);

                    // Collect context: 3 lines before and after selected
                    let start = app.file_tree_line.saturating_sub(3);
                    let end = (app.file_tree_line + 4).min(diff_lines.len());
                    let context: Vec<String> = diff_lines[start..end].to_vec();

                    app.compose_file_path = Some(path);
                    app.compose_line = actual_line;
                    app.compose_context = context;
                    app.compose_text.clear();
                    app.compose_cursor = 0;
                    app.navigate_to(Screen::ReviewCompose);
                }
            }
        }

        Action::FileTree => {
            if matches!(app.screen, Screen::PrDetail | Screen::ReviewCompose) {
                // Parse changed files from diff and populate checklist
                let diff_files: Vec<String> = if let Some(diff) = &app.current_diff {
                    diff.lines()
                        .filter(|l| l.starts_with("diff --git "))
                        .filter_map(|l| l.split(" b/").nth(1).map(str::to_string))
                        .collect()
                } else {
                    Vec::new()
                };
                if !diff_files.is_empty() {
                    let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
                    let draft = app.draft.get_or_insert_with(|| {
                        review::models::ReviewDraft::new(pr_num, review::models::ReviewMode::ManualOnly)
                    });
                    for f in diff_files {
                        draft.file_checklist.entry(f).or_insert(false);
                    }
                }
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
                // Auto-generate review body from approved comments
                if let Some(draft) = &mut app.draft {
                    let body = draft.generate_body();
                    draft.review_body = Some(body);
                }
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

        Action::Publish => {
            if app.screen == Screen::SummaryPreview {
                publish_review(app, event_tx, config).await;
            }
        }

        Action::NavLeft => {
            if app.screen == Screen::SummaryPreview && app.summary_event_idx > 0 {
                app.summary_event_idx -= 1;
            } else if app.screen == Screen::FileTree && app.file_tree_pane == 1 {
                app.file_tree_pane = 0;
            }
        }

        Action::NavRight => {
            if app.screen == Screen::SummaryPreview && app.summary_event_idx + 1 < 3 {
                app.summary_event_idx += 1;
            } else if app.screen == Screen::FileTree && app.file_tree_pane == 0 {
                app.file_tree_pane = 1;
                app.file_tree_line = 0;
            }
        }
    }
}

async fn handle_setup_action(
    app: &mut App,
    action: Action,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
) {
    use app::SetupField;

    match action {
        Action::Quit | Action::Back => {
            app.should_quit = true;
        }

        // Tab: switch between Owner and Repo fields
        Action::NextPane | Action::PrevPane => {
            app.setup_field = match app.setup_field {
                SetupField::Owner => SetupField::Repo,
                SetupField::Repo  => SetupField::Owner,
            };
        }

        // Enter: confirm and save
        Action::Confirm => {
            if app.setup_owner.trim().is_empty() || app.setup_repo.trim().is_empty() {
                app.show_error("Owner and repository name cannot be empty.");
                return;
            }
            app.setup_saving = true;

            let token = app.setup_gh_token.clone();
            let owner = app.setup_owner.trim().to_string();
            let repo  = app.setup_repo.trim().to_string();
            let tx    = event_tx.clone();

            tokio::spawn(async move {
                match config::AppConfig::save_github_config(&token, &owner, &repo) {
                    Ok(()) => {
                        let _ = tx.send(AppEvent::SetupSaved(token, owner, repo));
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::SetupFailed(format!("{:#}", e)));
                    }
                }
            });
        }

        // Typing: update the focused field
        Action::Char(c) => {
            match app.setup_field {
                SetupField::Owner => app.setup_owner.push(c),
                SetupField::Repo  => app.setup_repo.push(c),
            }
        }

        // Backspace
        Action::Delete => {
            match app.setup_field {
                SetupField::Owner => { app.setup_owner.pop(); }
                SetupField::Repo  => { app.setup_repo.pop(); }
            }
        }

        _ => {}
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
    app.diff_lines_cache = None;
    app.current_ticket = None;
    app.pr_loading = true;
    app.diff_loading = true;
    app.diff_scroll = 0;
    app.description_scroll = 0;
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

    // Load ticket (best-effort) — search title + branch for ticket keys
    {
        let tx = event_tx.clone();
        let cfg = config.clone();
        let pr_text = match app.selected_pr() {
            Some(pr) => format!("{} {}", pr.title, pr.head_branch),
            None => String::new(),
        };
        tokio::spawn(async move {
            let ticket = load_ticket_for_pr(&cfg, &pr_text).await;
            let _ = tx.send(AppEvent::TicketLoaded(ticket));
        });
    }
}

fn start_agent_runner(app: &mut App, config: &config::AppConfig) {
    use agents::context::ReviewContext;
    use agents::orchestrator::Orchestrator;
    use review::models::{ReviewDraft, ReviewMode};

    // If a previous review exists with comments, resume it instead of re-running agents
    if let Some(draft) = &app.draft {
        if !draft.comments.is_empty() {
            let n = draft.comments.len();
            let date = draft.started_at
                .format("%Y-%m-%d %H:%M UTC")
                .to_string();
            app.show_info(
                "Resuming Review",
                format!("Previous review started {date}\n{n} comment(s) found — resuming without re-running agents."),
            );
            app.navigate_to(Screen::DoubleCheck);
            return;
        }
    }

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

fn count_file_diff_lines(app: &App, path: &str) -> usize {
    app.current_diff.as_deref().map(|diff| {
        let target = format!("diff --git a/{} b/{}", path, path);
        let mut count = 0;
        let mut found = false;
        for line in diff.lines() {
            if line == target { found = true; continue; }
            if found && line.starts_with("diff --git ") { break; }
            if found { count += 1; }
        }
        count
    }).unwrap_or(0)
}

fn extract_file_diff_for_compose(app: &App, path: &str) -> Vec<String> {
    app.current_diff.as_deref().map(|diff| {
        let target = format!("diff --git a/{} b/{}", path, path);
        let mut result = Vec::new();
        let mut found = false;
        for line in diff.lines() {
            if line == target { found = true; }
            else if found && line.starts_with("diff --git ") { break; }
            if found { result.push(line.to_string()); }
        }
        result
    }).unwrap_or_default()
}

/// Compute the actual source line number for a given visual line index in a file diff.
fn get_diff_line_number(diff_lines: &[String], line_idx: usize) -> Option<u32> {
    let mut hunk_start: Option<u32> = None;
    let mut offset: u32 = 0;

    for (i, line) in diff_lines.iter().enumerate() {
        if line.starts_with("@@ ") {
            // Parse "+start" from "@@ -a,b +start,len @@"
            hunk_start = line.split('+').nth(1)
                .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
                .and_then(|s| s.parse::<u32>().ok());
            offset = 0;
            if i == line_idx { return hunk_start; }
        } else if hunk_start.is_some() {
            if i == line_idx {
                return hunk_start.map(|s| s + offset.saturating_sub(1));
            }
            if !line.starts_with('-') {
                offset += 1;
            }
        }
    }
    None
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
            app.compose_file_path.clone(),
            app.compose_line,
        );
        if let Some(draft) = &mut app.draft {
            draft.add_comment(comment);
            // Add to file_checklist if file not already there
            if let Some(ref path) = app.compose_file_path {
                draft.file_checklist.entry(path.clone()).or_insert(false);
            }
        }
    }
    app.compose_text.clear();
    app.compose_cursor = 0;
    app.compose_file_path = None;
    app.compose_line = None;
    app.compose_context.clear();
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

// ── Publish ────────────────────────────────────────────────────────────────

async fn publish_review(
    app: &mut App,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    config: &config::AppConfig,
) {
    use review::models::ReviewEvent;

    let draft = match &mut app.draft {
        Some(d) => d,
        None => {
            app.show_error("No review draft to publish.");
            return;
        }
    };

    // Sync the selected review event from the radio selector
    let selected_event = match app.summary_event_idx {
        0 => ReviewEvent::Comment,
        1 => ReviewEvent::RequestChanges,
        _ => ReviewEvent::Approve,
    };
    draft.review_event = selected_event;

    // Clone what we need before spawning
    let draft_clone = draft.clone();
    let tx = event_tx.clone();
    let cfg = config.clone();

    app.set_status("Publishing review…");

    tokio::spawn(async move {
        let result = async {
            let api = make_github_api(&cfg)?;
            let publisher = review::publisher::ReviewPublisher::new(api);
            publisher.publish(&draft_clone).await
        }
        .await;

        match result {
            Ok(()) => {
                let _ = tx.send(AppEvent::PublishDone);
            }
            Err(e) => {
                let _ = tx.send(AppEvent::PublishFailed(format!("{:#}", e)));
            }
        }
    });
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

/// Extract ticket keys from the PR text and resolve the first match.
/// Returns `None` silently on any error — ticket is always optional.
async fn load_ticket_for_pr(
    config: &config::AppConfig,
    pr_text: &str,
) -> Option<tickets::models::Ticket> {
    let providers = tickets::build_providers(config);
    if providers.is_empty() || pr_text.is_empty() {
        return None;
    }

    let keys = tickets::extractor::extract_ticket_keys(pr_text, &providers);
    if keys.is_empty() {
        return None;
    }

    tickets::extractor::resolve_ticket(&keys, &providers).await
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
