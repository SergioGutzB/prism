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

use crate::app::{App, PendingPublish, PopupKind, PopupState, Screen};
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

        // Fetch current authenticated user
        {
            let tx = event_tx.clone();
            let cfg = config.clone();
            tokio::spawn(async move {
                if let Ok(api) = make_github_api(&cfg) {
                    if let Ok(login) = api.get_current_user().await {
                        let _ = tx.send(AppEvent::UserLoaded(login));
                    }
                }
            });
        }
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
                        let display = if msg.contains("request changes on your own pull request") {
                            "Cannot request changes on your own PR. Change review type to 'Comment' or 'Approve'.".to_string()
                        } else {
                            format!("Publish failed: {}", msg)
                        };
                        app.show_error(display);
                    }

                    AppEvent::UserLoaded(login) => {
                        app.github_user = Some(login);
                    }

                    AppEvent::ReviewsLoaded(reviews, comments) => {
                        use review::models::{CommentSource, CommentStatus, GeneratedComment, Severity};
                        let draft = app.draft.get_or_insert_with(|| {
                            let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
                            review::models::ReviewDraft::new(pr_num, review::models::ReviewMode::ManualOnly)
                        });

                        // Add existing reviews as comments (only non-empty body reviews)
                        for review in &reviews {
                            if review.body.trim().is_empty() { continue; }
                            let severity = match review.state {
                                github::models::GhReviewState::ChangesRequested => Severity::Warning,
                                github::models::GhReviewState::Approved => Severity::Praise,
                                _ => Severity::Suggestion,
                            };
                            let mut comment = GeneratedComment::new(
                                CommentSource::Agent {
                                    agent_id: format!("gh_review_{}", review.id),
                                    agent_name: format!("@{}", review.user.login),
                                    agent_icon: "\u{1F50D}".to_string(),
                                },
                                review.body.clone(),
                                severity,
                                None,
                                None,
                            );
                            comment.status = CommentStatus::Approved;
                            let already = draft.comments.iter().any(|c| {
                                if let CommentSource::Agent { agent_id, .. } = &c.source {
                                    agent_id == &format!("gh_review_{}", review.id)
                                } else { false }
                            });
                            if !already { draft.comments.push(comment); }
                        }

                        // Add existing inline comments
                        for ic in &comments {
                            let mut comment = GeneratedComment::new(
                                CommentSource::Agent {
                                    agent_id: format!("gh_comment_{}", ic.id),
                                    agent_name: format!("@{}", ic.user.login),
                                    agent_icon: "\u{1F4AC}".to_string(),
                                },
                                ic.body.clone(),
                                Severity::Suggestion,
                                Some(ic.path.clone()),
                                ic.line,
                            );
                            comment.status = CommentStatus::Approved;
                            let already = draft.comments.iter().any(|c| {
                                if let CommentSource::Agent { agent_id, .. } = &c.source {
                                    agent_id == &format!("gh_comment_{}", ic.id)
                                } else { false }
                            });
                            if !already { draft.comments.push(comment); }
                        }

                        let total = reviews.iter().filter(|r| !r.body.trim().is_empty()).count() + comments.len();
                        if total > 0 {
                            app.set_status(format!("{} existing review(s) loaded from GitHub — press [v] to view", total));
                        }
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

                    AppEvent::QuickCommentDone => {
                        app.show_info("Published", "Comment posted to GitHub successfully.");
                        app.compose_text.clear();
                        app.compose_cursor = 0;
                        app.compose_quick_mode = false;
                        app.navigate_back();
                    }

                    AppEvent::QuickCommentFailed(msg) => {
                        app.show_error(format!("Failed to post comment: {}", msg));
                    }

                    AppEvent::ClaudeOutputDone(output) => {
                        app.claude_output = output;
                        app.claude_output_loading = false;
                        app.claude_output_scroll = 0;
                    }

                    AppEvent::ClaudeOutputFailed(msg) => {
                        app.claude_output_loading = false;
                        app.claude_output = format!("❌ Claude Code failed:\n\n{}", msg);
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

                if let Ok(size) = terminal.size() {
                    // Approximate diff viewport: full height minus header(3) + keybind(3) + borders(2)
                    app.diff_viewport_height = size.height.saturating_sub(8) as usize;
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
                let kind = app.popup.as_ref().map(|p| p.kind.clone());
                let pending = app.pending_publish.take();
                app.dismiss_popup();
                match kind {
                    Some(PopupKind::ConfirmQuit) => {
                        app.should_quit = true;
                    }
                    Some(PopupKind::ConfirmPublish) => {
                        match pending {
                            Some(PendingPublish::QuickComment { text }) => {
                                publish_quick_comment(app, event_tx, config, text).await;
                            }
                            Some(PendingPublish::FullReview) => {
                                publish_review(app, event_tx, config).await;
                            }
                            _ => {}
                        }
                    }
                    Some(PopupKind::ConfirmRestart) => {
                        if matches!(pending, Some(PendingPublish::RestartReview)) {
                            // Clear draft and re-run all agents from scratch
                            app.draft = None;
                            app.agent_statuses.clear();
                            start_agent_runner(app, config);
                        }
                    }
                    _ => {}
                }
            }
            Action::Quit | Action::Back | Action::ExitInsert => {
                let kind = app.popup.as_ref().map(|p| p.kind.clone());
                let pending = app.pending_publish.take();
                app.dismiss_popup();
                // On Esc from ConfirmRestart: navigate to DoubleCheck (resume)
                if kind == Some(PopupKind::ConfirmRestart) {
                    if matches!(pending, Some(PendingPublish::RestartReview)) {
                        if app.draft.as_ref().map(|d| !d.comments.is_empty()).unwrap_or(false) {
                            app.navigate_to(Screen::DoubleCheck);
                        }
                    }
                }
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
            // Dismiss overlays first
            if app.show_help {
                app.show_help = false;
                return;
            }
            if app.show_stats {
                app.show_stats = false;
                return;
            }
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
                    if app.double_check_pane == 1 {
                        // detail panel: scroll down
                        app.double_check_detail_scroll = app.double_check_detail_scroll.saturating_add(1);
                    } else {
                        let len = app.draft.as_ref().map(|d| d.comments.len()).unwrap_or(0);
                        if app.double_check_selected + 1 < len {
                            app.double_check_selected += 1;
                            app.double_check_detail_scroll = 0;
                        }
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
                            let max_scroll = max_lines.saturating_sub(app.diff_viewport_height);
                            app.diff_scroll = (app.diff_scroll + 3).min(max_scroll);
                        }
                    }
                }
                Screen::SummaryPreview => {
                    if app.summary_pane == 0 {
                        app.summary_body_scroll = app.summary_body_scroll.saturating_add(3);
                    } else {
                        app.summary_comments_scroll = app.summary_comments_scroll.saturating_add(1);
                    }
                }
                Screen::ClaudeCodeOutput => {
                    let max = app.claude_output.lines().count();
                    app.claude_output_scroll = (app.claude_output_scroll + 3).min(max);
                }
                _ => {}
            }
        }

        Action::NavUp => {
            match app.screen {
                Screen::PrList => app.nav_up(),
                Screen::DoubleCheck => {
                    if app.double_check_pane == 1 {
                        app.double_check_detail_scroll = app.double_check_detail_scroll.saturating_sub(1);
                    } else {
                        if app.double_check_selected > 0 {
                            app.double_check_selected -= 1;
                            app.double_check_detail_scroll = 0;
                        }
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
                Screen::SummaryPreview => {
                    if app.summary_pane == 0 {
                        app.summary_body_scroll = app.summary_body_scroll.saturating_sub(3);
                    } else {
                        app.summary_comments_scroll = app.summary_comments_scroll.saturating_sub(1);
                    }
                }
                Screen::ClaudeCodeOutput => {
                    app.claude_output_scroll = app.claude_output_scroll.saturating_sub(3);
                }
                _ => {}
            }
        }

        Action::GoTop => {
            match app.screen {
                Screen::DoubleCheck => {
                    if app.double_check_pane == 1 {
                        app.double_check_detail_scroll = 0;
                    } else {
                        app.double_check_selected = 0;
                    }
                }
                Screen::PrDetail => { app.diff_scroll = 0; }
                Screen::FileTree => { app.file_tree_scroll = 0; }
                _ => { app.pr_list_selected = 0; }
            }
        }

        Action::GoBottom => {
            match app.screen {
                Screen::DoubleCheck => {
                    if app.double_check_pane == 1 {
                        // Scroll detail to end — render will clamp to max
                        app.double_check_detail_scroll = usize::MAX / 2;
                    } else {
                        // Jump to last filtered comment
                        let last = filtered_comment_count(app).saturating_sub(1);
                        // Map filtered index back to real index
                        if let Some(draft) = &app.draft {
                            let comments = &draft.comments;
                            let count = comments.len();
                            if count > 0 {
                                let visible: Vec<usize> = (0..count)
                                    .filter(|&i| comment_passes_filter(app, i))
                                    .collect();
                                if let Some(&real_idx) = visible.get(last) {
                                    app.double_check_selected = real_idx;
                                }
                            }
                        }
                    }
                }
                Screen::PrDetail => {
                    let max = app.diff_lines_cache.as_ref().map(|l| l.len()).unwrap_or(0);
                    app.diff_scroll = max.saturating_sub(app.diff_viewport_height);
                }
                Screen::FileTree => {
                    app.file_tree_scroll = usize::MAX / 2; // clamped by render
                }
                Screen::ClaudeCodeOutput => {
                    let max = app.claude_output.lines().count();
                    app.claude_output_scroll = max;
                }
                _ => { app.go_bottom(); }
            }
        }

        Action::ScrollDown => {
            if app.screen == Screen::FileTree {
                app.file_tree_scroll = app.file_tree_scroll.saturating_add(5);
            } else {
                let max_lines = app.diff_lines_cache.as_ref().map(|l| l.len()).unwrap_or(0);
                let max_scroll = max_lines.saturating_sub(app.diff_viewport_height);
                app.diff_scroll = (app.diff_scroll + 10).min(max_scroll);
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
            let max_scroll = max_lines.saturating_sub(app.diff_viewport_height);
            app.diff_scroll = (app.diff_scroll + 25).min(max_scroll);
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
            if app.screen == Screen::SummaryPreview {
                app.summary_pane = (app.summary_pane + 1) % 2;
            } else if app.screen == Screen::DoubleCheck {
                app.double_check_pane = (app.double_check_pane + 1) % 2;
            } else {
                app.selected_pane = (app.selected_pane + 1) % 3;
            }
        }

        Action::PrevPane => {
            if app.screen == Screen::SummaryPreview {
                app.summary_pane = if app.summary_pane == 0 { 1 } else { 0 };
            } else if app.screen == Screen::DoubleCheck {
                app.double_check_pane = if app.double_check_pane == 0 { 1 } else { 0 };
            } else {
                app.selected_pane = if app.selected_pane == 0 { 2 } else { app.selected_pane - 1 };
            }
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
                    // newline
                    let pos = app.compose_cursor.min(app.compose_text.len());
                    app.compose_text.insert(pos, '\n');
                    app.compose_cursor += 1;
                } else {
                    let text = app.compose_text.trim().to_string();
                    if text.is_empty() {
                        app.show_error("Comment cannot be empty.");
                    } else if app.compose_quick_mode {
                        // Quick comment: show confirmation popup, store pending action
                        app.pending_publish = Some(crate::app::PendingPublish::QuickComment { text });
                        app.popup = Some(crate::app::PopupState {
                            title: "Publish Comment".to_string(),
                            message: format!("Post comment to PR #{}?\n\nThis will be published immediately as a PR conversation comment.", app.current_pr.as_ref().map(|p| p.number).unwrap_or(0)),
                            kind: crate::app::PopupKind::ConfirmPublish,
                        });
                    } else {
                        // Inline review comment: save to draft and go to DoubleCheck
                        save_compose_comment(app);
                        app.navigate_to(Screen::DoubleCheck);
                    }
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
            if !config.is_llm_configured() {
                app.show_info(
                    "AI Unavailable",
                    "Claude CLI not found and no API key configured.\nInstall Claude Code or set ANTHROPIC_API_KEY.",
                );
            } else if app.screen == Screen::DoubleCheck {
                // From DoubleCheck: run any agents not yet completed (append mode)
                run_missing_agents(app, config);
            } else if app.screen == Screen::PrDetail {
                start_agent_runner(app, config);
            }
        }

        Action::RunMissingAgents => {
            if config.is_llm_configured() && app.screen == Screen::DoubleCheck {
                run_missing_agents(app, config);
            }
        }

        Action::ClaudeCodeFix => {
            if app.screen == Screen::DoubleCheck {
                send_to_claude_code(app, event_tx, config);
            } else if app.screen == Screen::ClaudeCodeOutput {
                // jk scrolling handled by NavUp/NavDown — nothing extra needed here
            }
        }

        Action::RestartReview => {
            if config.is_llm_configured() {
                let n = app.draft.as_ref().map(|d| d.comments.len()).unwrap_or(0);
                app.pending_publish = Some(crate::app::PendingPublish::RestartReview);
                app.popup = Some(crate::app::PopupState {
                    title: "Restart Review".to_string(),
                    message: format!(
                        "Clear all {} existing comment(s) and re-run all enabled agents?\n\nThis cannot be undone.",
                        n
                    ),
                    kind: crate::app::PopupKind::ConfirmRestart,
                });
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
                app.compose_quick_mode = true;
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
                    app.compose_quick_mode = false;
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

        Action::OpenDoubleCheck => {
            if matches!(app.screen, Screen::PrDetail | Screen::FileTree | Screen::ReviewCompose) {
                // Ensure draft exists so DoubleCheck has somewhere to land
                let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
                app.draft.get_or_insert_with(|| {
                    review::models::ReviewDraft::new(pr_num, review::models::ReviewMode::ManualOnly)
                });
                app.navigate_to(Screen::DoubleCheck);
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
                // Navigate to preview — body stays as-is (empty or previously generated)
                app.summary_body_scroll = 0;
                app.summary_comments_scroll = 0;
                app.summary_pane = 0;
                app.navigate_to(Screen::SummaryPreview);
            }
        }

        Action::GenerateBody => {
            if app.screen == Screen::SummaryPreview {
                if let Some(draft) = &mut app.draft {
                    let body = draft.generate_body_with_format(&app.config.publishing.format);
                    draft.review_body = Some(body);
                    app.summary_body_scroll = 0;
                }
                app.set_status("Review body generated.");
            }
        }

        Action::InsertSuggestion => {
            if app.screen == Screen::ReviewCompose && app.input_mode != InputMode::Insert {
                // Build a suggestion block pre-filled with the current line's code
                let current_code = app.compose_context
                    .get(app.compose_context.len() / 2)
                    .map(|l| if l.len() > 1 { l[1..].to_string() } else { String::new() })
                    .unwrap_or_default();
                let suggestion = format!("```suggestion\n{}\n```", current_code);
                app.compose_text = suggestion;
                app.compose_cursor = app.compose_text.len();
                app.input_mode = InputMode::Normal;
                app.set_status("Suggestion template inserted — edit the code block, then [Enter] to add.");
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
                let event_name = match app.summary_event_idx {
                    0 => "COMMENT",
                    1 => "REQUEST_CHANGES",
                    _ => "APPROVE",
                };
                let n = app.draft.as_ref().map(|d| d.submittable_count()).unwrap_or(0);
                app.pending_publish = Some(crate::app::PendingPublish::FullReview);
                app.popup = Some(crate::app::PopupState {
                    title: "Submit Review".to_string(),
                    message: format!("Submit review to PR #{}?\n\nType: {}\nInline comments: {}",
                        app.current_pr.as_ref().map(|p| p.number).unwrap_or(0),
                        event_name, n),
                    kind: crate::app::PopupKind::ConfirmPublish,
                });
            }
        }

        Action::Help => {
            app.show_help = !app.show_help;
            app.show_stats = false; // close stats if open
        }

        Action::ShowStats => {
            app.show_stats = !app.show_stats;
            app.show_help = false; // close help if open
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

    // Accumulate token stats when an agent completes
    if let agents::models::AgentStatus::Done { input_tokens, output_tokens, .. } = &update.status {
        app.token_input_total += input_tokens;
        app.token_output_total += output_tokens;
        app.token_calls_total += 1;
    }

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

    // Load existing reviews and inline comments from GitHub
    {
        let tx = event_tx.clone();
        let cfg = config.clone();
        tokio::spawn(async move {
            let api = match make_github_api(&cfg) {
                Ok(a) => a,
                Err(_) => return,
            };
            let reviews = api.list_reviews(pr_num).await.unwrap_or_default();
            let comments = api.list_inline_comments(pr_num).await.unwrap_or_default();
            let _ = tx.send(AppEvent::ReviewsLoaded(reviews, comments));
        });
    }
}

fn start_agent_runner(app: &mut App, config: &config::AppConfig) {
    use agents::context::ReviewContext;
    use agents::orchestrator::Orchestrator;
    use review::models::{ReviewDraft, ReviewMode};

    // If a previous review exists with comments, ask user what to do
    if let Some(draft) = &app.draft {
        if !draft.comments.is_empty() {
            let n = draft.comments.len();
            let date = draft.started_at.format("%Y-%m-%d %H:%M UTC").to_string();
            app.pending_publish = Some(crate::app::PendingPublish::RestartReview);
            app.popup = Some(crate::app::PopupState {
                title: "Review In Progress".to_string(),
                message: format!(
                    "Review started {date} — {n} comment(s) already generated.\n\n\
                     [Enter] Restart from scratch (clear all comments)\n\
                     [Esc]   Resume existing (go to Double-Check)\n\n\
                     To run only missing agents, press [Esc] then [r] in Double-Check."
                ),
                kind: crate::app::PopupKind::ConfirmRestart,
            });
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

/// Run only agents that haven't completed yet (no Done/Disabled status), appending results to
/// the existing draft instead of replacing it.
fn run_missing_agents(app: &mut App, config: &config::AppConfig) {
    use agents::context::ReviewContext;
    use agents::models::AgentStatus;
    use agents::orchestrator::Orchestrator;
    use review::models::{ReviewDraft, ReviewMode};

    let pr = match &app.current_pr {
        Some(pr) => pr.clone(),
        None => {
            app.show_error("No PR loaded — open a PR first.");
            return;
        }
    };

    // Determine which agents still need to run
    let missing: Vec<_> = app.agents
        .iter()
        .filter(|a| {
            if !a.agent.enabled { return false; }
            match app.agent_statuses.get(&a.agent.id) {
                Some(AgentStatus::Done { .. }) | Some(AgentStatus::Disabled) => false,
                _ => true, // Pending, Running, Failed, or not started
            }
        })
        .cloned()
        .collect();

    if missing.is_empty() {
        app.show_info(
            "All Agents Done",
            "All enabled agents have already completed.\n\nUse [R] to restart from scratch.",
        );
        return;
    }

    let diff = app.current_diff.clone().unwrap_or_default();
    let ticket = app.current_ticket.clone();
    let ctx = ReviewContext::from_pr(&pr, &diff, ticket);

    // Keep existing draft or create one; keep existing comments
    let pr_num = pr.number;
    if app.draft.is_none() {
        app.draft = Some(ReviewDraft::new(pr_num, ReviewMode::AiOnly));
    }

    // Mark missing agents as Pending in status map
    for agent in &missing {
        app.agent_statuses.insert(
            agent.agent.id.clone(),
            agents::models::AgentStatus::Pending,
        );
    }

    let n = missing.len();
    app.navigate_to(Screen::AgentRunner);
    app.set_status(format!("Running {} missing agent(s)…", n));

    let orchestrator = Orchestrator::new(config.clone());
    let rx = orchestrator.run_all(missing, ctx);
    app.agent_rx = Some(rx);
}

fn count_file_diff_lines(app: &App, path: &str) -> usize {
    app.current_diff.as_deref().map(|diff| {
        let target = format!("diff --git a/{} b/{}", path, path);
        let mut count = 0;
        let mut found = false;
        for line in diff.lines() {
            if line == target { found = true; }          // include the header line in count
            else if found && line.starts_with("diff --git ") { break; }
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

/// Returns how many comments pass the current agent filter.
fn filtered_comment_count(app: &App) -> usize {
    match &app.draft {
        None => 0,
        Some(d) => (0..d.comments.len())
            .filter(|&i| comment_passes_filter(app, i))
            .count(),
    }
}

/// Returns true if the comment at `idx` passes the current agent filter.
fn comment_passes_filter(app: &App, idx: usize) -> bool {
    use review::models::CommentSource;
    let filter = match app.agent_filter {
        None => return true,
        Some(f) => f,
    };
    let comment = match app.draft.as_ref().and_then(|d| d.comments.get(idx)) {
        Some(c) => c,
        None => return false,
    };
    match &comment.source {
        CommentSource::Agent { agent_id, .. } => {
            let idx_1based = app.agents.iter()
                .position(|a| a.agent.id == *agent_id)
                .map(|i| i as u8 + 1)
                .unwrap_or(0);
            idx_1based == filter
        }
        CommentSource::Manual => filter == 0,
    }
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

// ── Claude Code integration ─────────────────────────────────────────────────

/// Build the review task prompt from all non-rejected comments in the draft.
fn build_claude_fix_prompt(app: &App) -> String {
    use review::models::CommentStatus;
    let pr = app.current_pr.as_ref();
    let pr_num = pr.map(|p| p.number).unwrap_or(0);
    let pr_title = pr.map(|p| p.title.as_str()).unwrap_or("Unknown PR");

    let mut prompt = format!(
        "You are a code-review assistant. Below are review comments that need to be applied \
         to the codebase. For each comment, implement the suggested change as a concrete code \
         edit. Show the exact file path, line numbers, and the corrected code.\n\n\
         ## PR #{pr_num}: {pr_title}\n\n"
    );

    let draft = match &app.draft {
        Some(d) => d,
        None => {
            prompt.push_str("No review comments found.");
            return prompt;
        }
    };

    let submittable: Vec<_> = draft.comments.iter()
        .filter(|c| c.status != CommentStatus::Rejected)
        .collect();

    if submittable.is_empty() {
        prompt.push_str("No comments to process (all were rejected).");
        return prompt;
    }

    for (i, comment) in submittable.iter().enumerate() {
        use review::models::CommentSource;
        let source = match &comment.source {
            CommentSource::Agent { agent_name, .. } => agent_name.clone(),
            CommentSource::Manual => "Manual".to_string(),
        };
        let location = match (&comment.file_path, comment.line) {
            (Some(f), Some(l)) => format!("{f}:{l}"),
            (Some(f), None) => f.clone(),
            _ => "(general)".to_string(),
        };
        prompt.push_str(&format!(
            "### Comment {} — [{:?}] @ {}\nSource: {}\n\n{}\n\n---\n\n",
            i + 1,
            comment.severity,
            location,
            source,
            comment.effective_body(),
        ));
    }

    prompt.push_str(
        "For each comment above, provide:\n\
         1. The exact file and line(s) to change\n\
         2. The corrected code\n\
         3. A brief explanation of the fix\n"
    );
    prompt
}

/// Spawn a Claude Code subprocess with the review task and send results via event channel.
fn send_to_claude_code(
    app: &mut App,
    tx: &tokio::sync::mpsc::UnboundedSender<tui::event::AppEvent>,
    config: &config::AppConfig,
) {
    let prompt = build_claude_fix_prompt(app);
    let tx = tx.clone();
    let llm = config.llm.clone();
    let timeout = config.agents.timeout_secs.max(120); // at least 2 min for this task

    app.claude_output = String::new();
    app.claude_output_scroll = 0;
    app.claude_output_loading = true;
    app.navigate_to(Screen::ClaudeCodeOutput);

    tokio::spawn(async move {
        let system = "You are a senior software engineer performing code review corrections. \
            Respond with concrete, actionable code changes. Be precise about file paths and line numbers.";

        let client = reqwest::Client::new();
        let result = agents::runner::call_provider(
            &client,
            &llm,
            &llm.model.clone(),
            llm.temperature,
            llm.max_tokens,
            system,
            &prompt,
            timeout,
            "ai-fix",
        )
        .await;

        match result {
            Ok(text) => { let _ = tx.send(tui::event::AppEvent::ClaudeOutputDone(text)); }
            Err(e)   => { let _ = tx.send(tui::event::AppEvent::ClaudeOutputFailed(format!("{:#}", e))); }
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

// ── Quick comment publish ──────────────────────────────────────────────────

async fn publish_quick_comment(
    app: &mut App,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    config: &config::AppConfig,
    text: String,
) {
    let pr_num = match app.current_pr.as_ref().map(|p| p.number) {
        Some(n) => n,
        None => {
            app.show_error("No PR loaded.");
            return;
        }
    };

    // If auto-translate is enabled and LLM available, translate first
    let final_text = if config.publishing.auto_translate_to_english && config.is_llm_configured() {
        app.set_status("Translating to English…");
        match translate_to_english(&text, config).await {
            Ok(translated) => translated,
            Err(_) => text, // fall back to original on error
        }
    } else if config.publishing.auto_correct_grammar && config.is_llm_configured() {
        app.set_status("Correcting grammar…");
        match correct_grammar(&text, config).await {
            Ok(corrected) => corrected,
            Err(_) => text,
        }
    } else {
        text
    };

    let tx = event_tx.clone();
    let cfg = config.clone();
    app.set_status("Publishing comment…");
    tokio::spawn(async move {
        let result = async {
            let api = make_github_api(&cfg)?;
            api.post_pr_comment(pr_num, &final_text).await
        }.await;
        match result {
            Ok(()) => { let _ = tx.send(AppEvent::QuickCommentDone); }
            Err(e) => { let _ = tx.send(AppEvent::QuickCommentFailed(format!("{:#}", e))); }
        }
    });
}

async fn translate_to_english(text: &str, config: &config::AppConfig) -> anyhow::Result<String> {
    call_llm_for_text(
        "You are a technical writing assistant. Translate the following text to English. Return ONLY the translated text, nothing else.",
        text,
        config,
    ).await
}

async fn correct_grammar(text: &str, config: &config::AppConfig) -> anyhow::Result<String> {
    call_llm_for_text(
        "You are a technical writing assistant. Correct the grammar and spelling of the following text while preserving its meaning and technical terminology. Keep code blocks unchanged. Return ONLY the corrected text, nothing else.",
        text,
        config,
    ).await
}

async fn call_llm_for_text(system: &str, text: &str, config: &config::AppConfig) -> anyhow::Result<String> {
    use anyhow::Context;
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;
    use std::process::Stdio;

    let timeout_secs = config.agents.timeout_secs;
    let mut child = Command::new("claude")
        .arg("--print")
        .arg("--system-prompt")
        .arg(system)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn claude")?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes()).await?;
    }

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("LLM translation timed out"))??;

    if !output.status.success() {
        anyhow::bail!("claude exited with error");
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
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
