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
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

use crate::app::{App, PendingPublish, PopupKind, PopupState, Screen};
use crate::tui::event::{spawn_event_reader, AppEvent};
use crate::tui::keybindings::{map_key, Action, InputMode, KeySequence};

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    let _guard = init_tracing();

    info!("prism v{}", env!("CARGO_PKG_VERSION"));

    let config = match config::AppConfig::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load configuration: {e}");
            std::process::exit(1);
        }
    };

    let agents = match agents::loader::load_agents(&config) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Warning: failed to load agents: {e}");
            vec![]
        }
    };

    let mut app = App::new(config.clone(), agents);
    app.model_stats = config::AppConfig::load_stats();

    let mut terminal = tui::terminal::init()?;
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();
    spawn_event_reader(event_tx.clone());

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

        let tx = event_tx.clone();
        let cfg = config.clone();
        tokio::spawn(async move {
            if let Ok(api) = make_github_api(&cfg).await {
                if let Ok(login) = api.get_current_user().await {
                    let _ = tx.send(AppEvent::UserLoaded(login));
                }
            }
        });
    } else {
        let gh_token = config::AppConfig::gh_token();
        if let Some(token) = gh_token {
            let (owner, repo) = config::AppConfig::gh_current_repo().unwrap_or_default();
            app.setup_gh_token = token;
            app.setup_owner = owner;
            app.setup_repo = repo;
        }
        app.screen = Screen::Setup;
        app.pr_list_loading = false;
    }

    let mut render_ticker = tokio::time::interval(std::time::Duration::from_millis(16));
    render_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            maybe_event = event_rx.recv() => {
                let event = match maybe_event { Some(e) => e, None => break };
                match event {
                    AppEvent::Key(key_event) => {
                        // Bypass key sequence detector when an editor is in insert mode.
                        // Check editor.is_insert_mode directly to handle the very first key after 'i',
                        // since app.input_mode might lag one event behind the editor's internal state.
                        let bypass_seq_detector = app.input_mode == InputMode::Insert
                            || match app.screen {
                                Screen::ReviewCompose => app.compose_editor.is_insert_mode,
                                Screen::AgentWizard
                                    if app.wizard_field == crate::app::AgentWizardField::SystemPrompt =>
                                {
                                    app.wizard_prompt_editor.is_insert_mode
                                }
                                // Simple wizard fields always accept text — bypass seq detector
                                Screen::AgentWizard => true,
                                _ => false,
                            };

                        if !bypass_seq_detector {
                            if let Some(seq) = app.key_detector.feed(&key_event) {
                                handle_sequence(&mut app, seq);
                                if app.should_quit { break; }
                                continue;
                            }
                        }

                        let captured = match app.screen {
                            Screen::ReviewCompose => {
                                let handled = app.compose_editor.handle_key(key_event);
                                app.input_mode = if app.compose_editor.is_insert_mode {
                                    InputMode::Insert
                                } else {
                                    InputMode::Normal
                                };
                                handled
                            }
                            // Simple wizard text fields: always accept Char/Backspace directly,
                            // no Insert mode ceremony needed.
                            Screen::AgentWizard
                                if app.wizard_field != crate::app::AgentWizardField::SystemPrompt =>
                            {
                                use crossterm::event::{KeyCode, KeyModifiers};
                                match key_event.code {
                                    KeyCode::Char(c)
                                        if !key_event.modifiers.contains(KeyModifiers::CONTROL) =>
                                    {
                                        match app.wizard_field {
                                            crate::app::AgentWizardField::Id => { app.wizard_id.push(c); }
                                            crate::app::AgentWizardField::Name => { app.wizard_name.push(c); }
                                            crate::app::AgentWizardField::Icon => {
                                                app.wizard_icon.clear();
                                                app.wizard_icon.push(c);
                                            }
                                            crate::app::AgentWizardField::SystemPrompt => {}
                                        }
                                        true
                                    }
                                    KeyCode::Backspace => {
                                        match app.wizard_field {
                                            crate::app::AgentWizardField::Id => { app.wizard_id.pop(); }
                                            crate::app::AgentWizardField::Name => { app.wizard_name.pop(); }
                                            crate::app::AgentWizardField::Icon => { app.wizard_icon.pop(); }
                                            crate::app::AgentWizardField::SystemPrompt => {}
                                        }
                                        true
                                    }
                                    _ => false,
                                }
                            }
                            Screen::AgentWizard
                                if app.wizard_field == crate::app::AgentWizardField::SystemPrompt =>
                            {
                                let handled = app.wizard_prompt_editor.handle_key(key_event);
                                app.input_mode = if app.wizard_prompt_editor.is_insert_mode {
                                    InputMode::Insert
                                } else {
                                    InputMode::Normal
                                };
                                handled
                            }
                            _ => false,
                        };

                        if !captured {
                            if let Some(action) = map_key(&key_event, &app.input_mode) {
                                handle_action(&mut app, action, &event_tx, &config).await;
                            }
                        }
                        if app.should_quit { break; }
                    }
                    AppEvent::PrListLoaded(prs) => {
                        // Prune drafts (and cache) for PRs that are no longer open
                        let open_nums: Vec<u64> = prs.iter().map(|p| p.number).collect();
                        let repo_slug = format!("{}/{}", config.github.owner, config.github.repo);
                        review::cache::prune_closed(&repo_slug, &open_nums);
                        prune_closed_drafts(&repo_slug, &open_nums);
                        app.pr_list = prs;
                        app.pr_list_loading = false;
                    }
                    AppEvent::PrLoaded(pr) => {
                        let t = ui::theme::Theme::current(&app.config.ui.theme);
                        app.pr_description_md_cache = Some(ui::components::markdown::parse(&pr.body, &t));
                        app.current_pr = Some(*pr);
                        app.pr_loading = false;
                    }
                    AppEvent::DiffLoaded(diff) => {
                        app.diff_line_ext = build_diff_line_ext(&diff);
                        let lines: Vec<String> = diff.lines().map(str::to_string).collect();
                        app.split_diff_cache = Some(ui::components::diff_view::parse_to_split(&lines));
                        app.diff_lines_cache = Some(lines);
                        app.current_diff = Some(diff);
                        app.diff_loading = false;
                        app.diff_cursor = 0;
                        app.diff_scroll = 0;
                        // Ensure a draft exists (handles race where DiffLoaded arrives before ReviewsLoaded)
                        if app.draft.is_none() {
                            let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
                            if pr_num > 0 {
                                app.draft = Some(review::models::ReviewDraft::new(pr_num, review::models::ReviewMode::ManualOnly));
                            }
                        }
                        if let (Some(diff_str), Some(draft)) = (app.current_diff.as_ref(), app.draft.as_mut()) {
                            populate_checklist_from_diff(draft, diff_str);
                        }
                        // Restore the persisted draft (all comments + checklist state).
                        // We merge rather than replace so that comments already added by a
                        // preceding ReviewsLoaded (race) are not lost.
                        let repo_slug = format!("{}/{}", config.github.owner, config.github.repo);
                        if let Some(pr_num) = app.draft.as_ref().map(|d| d.pr_number) {
                            if let Some(saved) = review::draft_store::load(pr_num, &repo_slug) {
                                if let Some(draft) = &mut app.draft {
                                    // Keep any comments already in draft (e.g. from ReviewsLoaded
                                    // arriving before us) that are not in the saved file, then
                                    // start from the saved set and add_comment the in-memory ones.
                                    let in_memory = std::mem::take(&mut draft.comments);
                                    draft.comments = saved.comments;
                                    draft.review_body = saved.review_body;
                                    draft.review_event = saved.review_event;
                                    draft.mode = saved.mode;
                                    // Re-merge any comments that arrived before us (dedup by github_id)
                                    for c in in_memory {
                                        draft.add_comment(c);
                                    }
                                    // Restore check marks for files that still exist in the diff
                                    for (path, checked) in &saved.file_checklist {
                                        if let Some(entry) = draft.file_checklist.get_mut(path) {
                                            *entry = *checked;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    AppEvent::ConfigReloaded(cfg, agents) => {
                        app.config = cfg;
                        app.agents = agents;
                        // Theme may have changed — invalidate the markdown cache so it's
                        // re-rendered with the new colors on the next PrLoaded or next open.
                        if let Some(pr) = &app.current_pr {
                            let t = ui::theme::Theme::current(&app.config.ui.theme);
                            app.pr_description_md_cache = Some(ui::components::markdown::parse(&pr.body, &t));
                        }
                        app.set_status("Config reloaded.");
                    }
                    AppEvent::Error(msg) => { app.show_error(msg); }
                    AppEvent::AgentUpdate(update) => { handle_agent_update(&mut app, update, &config); }
                    AppEvent::UserLoaded(login) => { app.github_user = Some(login); }
                    AppEvent::ReviewsLoaded(for_pr_num, reviews, comments) => {
                        // Discard if the user has already switched to a different PR
                        let current_num = app.current_pr.as_ref().map(|p| p.number);
                        let matches = current_num == Some(for_pr_num)
                            || (current_num.is_none() && app.pr_loading);
                        if matches && (!reviews.is_empty() || !comments.is_empty()) {
                            let draft = app.draft.get_or_insert_with(|| {
                                review::models::ReviewDraft::new(for_pr_num, review::models::ReviewMode::ManualOnly)
                            });
                            // Only merge if the draft belongs to this PR
                            if draft.pr_number == for_pr_num || draft.pr_number == 0 {
                                draft.pr_number = for_pr_num;
                                draft.merge_github_reviews(reviews, comments);
                                if let Some(diff) = &app.current_diff.clone() {
                                    populate_checklist_from_diff(draft, diff);
                                }
                            }
                            save_draft(&app, &config);
                        }
                    }
                    AppEvent::PublishDone => {
                        // Delete the draft — it has been submitted to GitHub
                        if let Some(draft) = &app.draft {
                            let repo_slug = format!("{}/{}", config.github.owner, config.github.repo);
                            review::draft_store::delete(draft.pr_number, &repo_slug);
                        }
                        app.draft = None;
                        app.set_status("Review published successfully!");
                        app.navigate_to(Screen::PrList);
                    }
                    AppEvent::PublishFailed(e) => { app.show_error(format!("Publish failed: {e}")); }
                    AppEvent::CommentDeleted(_id) => {
                        app.set_status("Comment deleted from GitHub.");
                        save_draft(&app, &config);
                    }
                    AppEvent::CommentUpdated(id, new_body) => {
                        if let Some(draft) = &mut app.draft {
                            if let Some(c) = draft.comments.iter_mut().find(|c| c.id == id) {
                                c.body = new_body;
                                c.edited_body = None;
                            }
                        }
                        app.set_status("Comment updated on GitHub.");
                        save_draft(&app, &config);
                    }
                    AppEvent::ConventionsLoaded(conventions) => {
                        app.project_conventions = conventions;
                    }
                    AppEvent::FixTaskChunk(idx, chunk) => {
                        if let Some(task) = app.fix_tasks.iter_mut().find(|t| t.index == idx) {
                            task.output.push_str(&chunk);
                            task.status = app::FixTaskStatus::Running;
                        }
                    }
                    AppEvent::FixTaskDone(idx) => {
                        if let Some(task) = app.fix_tasks.iter_mut().find(|t| t.index == idx) {
                            task.status = app::FixTaskStatus::Done;
                        }
                        app.ai_fix_loading = app.fix_tasks.iter()
                            .any(|t| matches!(t.status, app::FixTaskStatus::Pending | app::FixTaskStatus::Running));
                    }
                    AppEvent::FixTaskFailed(idx, err) => {
                        if let Some(task) = app.fix_tasks.iter_mut().find(|t| t.index == idx) {
                            task.status = app::FixTaskStatus::Failed(err);
                        }
                        app.ai_fix_loading = app.fix_tasks.iter()
                            .any(|t| matches!(t.status, app::FixTaskStatus::Pending | app::FixTaskStatus::Running));
                    }
                    AppEvent::ReviewBodyGenerated(body) => {
                        app.review_body_generating = false;
                        if let Some(draft) = &mut app.draft {
                            draft.review_body = Some(body);
                        }
                        save_draft(&app, &config);
                    }
                    AppEvent::ReviewBodyFailed(err) => {
                        app.review_body_generating = false;
                        app.show_error(format!("Failed to generate review body: {err}"));
                    }
                    _ => {}
                }
            }
            agent_update = async {
                if let Some(rx) = &mut app.agent_rx {
                    rx.recv().await
                } else {
                    std::future::pending::<Option<agents::orchestrator::AgentUpdate>>().await
                }
            } => {
                match agent_update {
                    Some(update) => handle_agent_update(&mut app, update, &config),
                    None => { app.agent_rx = None; } // channel closed
                }
            }
            _ = render_ticker.tick() => {
                app.tick = app.tick.wrapping_add(1);
                if let Ok(size) = terminal.size() {
                    app.diff_viewport_height = size.height.saturating_sub(8) as usize;
                }
                terminal.draw(|frame| ui::render(frame, &app))?;
            }
        }
    }

    tui::terminal::restore(&mut terminal)?;
    config::AppConfig::save_stats(&app.model_stats);
    Ok(())
}

async fn handle_action(
    app: &mut App,
    action: Action,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    config: &config::AppConfig,
) {
    // ── 1. Overlays Hierarchy (Help/Stats) ──────────────────────────────────
    if app.show_help {
        if action == Action::Back || action == Action::Help {
            app.show_help = false;
        }
        return;
    }
    if app.show_stats {
        match action {
            Action::Back | Action::ShowStats => {
                app.show_stats = false;
            }
            Action::NavRight | Action::NavDown => {
                app.stats_range = (app.stats_range + 1) % 4;
            }
            Action::NavLeft | Action::NavUp => {
                app.stats_range = if app.stats_range == 0 {
                    3
                } else {
                    app.stats_range - 1
                };
            }
            _ => {}
        }
        return;
    }

    // ── 2. Popups (Blocking) ────────────────────────────────────────────────
    if let Some(popup) = &app.popup {
        let kind = popup.kind.clone();

        // ConfirmCancelAgents has three options
        if kind == PopupKind::ConfirmCancelAgents {
            match action {
                Action::Back => {
                    // [Esc] — stay on AgentRunner, dismiss modal
                    app.dismiss_popup();
                }
                Action::Confirm => {
                    // [Enter] — cancel jobs and go back
                    app.dismiss_popup();
                    if let Some(abort) = app.agent_abort.take() {
                        abort.abort();
                    }
                    app.agent_rx = None;
                    app.navigate_back();
                }
                Action::ManualComment => {
                    // [c] — go back but let jobs continue; results will appear when done
                    app.dismiss_popup();
                    app.navigate_back();
                }
                _ => {}
            }
            return;
        }

        if action == Action::Back {
            app.dismiss_popup();
            return;
        }
        if action == Action::Confirm {
            let pending = app.pending_publish.take();
            let delete_id = app.pending_delete_comment.take();
            app.dismiss_popup();
            match kind {
                PopupKind::ConfirmQuit => {
                    app.should_quit = true;
                }
                PopupKind::ConfirmPublish => {
                    if let Some(crate::app::PendingPublish::FullReview) = pending {
                        publish_review(app, event_tx, config).await;
                    }
                }
                PopupKind::ConfirmRestart => {
                    // Delete persisted draft so a fresh run starts clean
                    if let Some(pr_num) = app.current_pr.as_ref().map(|p| p.number) {
                        let repo_slug = format!("{}/{}", config.github.owner, config.github.repo);
                        review::draft_store::delete(pr_num, &repo_slug);
                    }
                    app.draft = None;
                    start_agent_runner(app, config);
                }
                PopupKind::ConfirmDeleteComment => {
                    if let Some(local_id) = delete_id {
                        use review::models::CommentSource;
                        // Look up github_id + source type before removing
                        let (github_id, is_review_summary) = app
                            .draft
                            .as_ref()
                            .and_then(|d| d.comments.iter().find(|c| c.id == local_id))
                            .map(|c| {
                                (
                                    c.github_id,
                                    matches!(c.source, CommentSource::GithubReview { .. }),
                                )
                            })
                            .unwrap_or((None, false));
                        let pr_num = app.current_pr.as_ref().map(|p| p.number);
                        // Remove from local draft immediately
                        if let Some(draft) = &mut app.draft {
                            draft.comments.retain(|c| c.id != local_id);
                            let total = draft.comments.len();
                            if app.double_check_selected >= total && total > 0 {
                                app.double_check_selected = total - 1;
                            }
                        }
                        // Spawn remote deletion if this came from GitHub
                        if let Some(gh_id) = github_id {
                            let tx = event_tx.clone();
                            let cfg = config.clone();
                            tokio::spawn(async move {
                                if let Ok(api) = make_github_api(&cfg).await {
                                    let result = if is_review_summary {
                                        api.delete_review(pr_num.unwrap_or(0), gh_id).await
                                    } else {
                                        api.delete_review_comment(gh_id).await
                                    };
                                    match result {
                                        Ok(_) => {
                                            let _ = tx.send(AppEvent::CommentDeleted(local_id));
                                        }
                                        Err(e) => {
                                            let _ = tx.send(AppEvent::Error(format!(
                                                "Delete failed: {e}"
                                            )));
                                        }
                                    }
                                }
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        return;
    }

    // ── 3. Main Action Logic ────────────────────────────────────────────────
    match action {
        Action::Quit => {
            app.popup = Some(PopupState {
                title: "Quit".to_string(),
                message: "Are you sure you want to quit?".to_string(),
                kind: PopupKind::ConfirmQuit,
            });
        }
        Action::Back => {
            if app.input_mode == InputMode::Insert {
                app.input_mode = InputMode::Normal;
            } else if app.screen == Screen::AgentRunner && app.agent_abort.is_some() && !app.agents_committed {
                // Jobs are still running — ask what to do
                app.popup = Some(PopupState {
                    title: "Review in progress".to_string(),
                    message: "AI agents are still running.\n\nCancel the jobs, or go back and let them finish in the background?".to_string(),
                    kind: PopupKind::ConfirmCancelAgents,
                });
            } else {
                if app.screen == Screen::PrDetail {
                    app.diff_fullscreen = false;
                    app.diff_split_mode = false;
                    app.diff_scroll = 0;
                } else if app.screen == Screen::FileTree {
                    if app.file_tree_fullscreen {
                        // First Esc exits fullscreen, second goes back
                        app.file_tree_fullscreen = false;
                        app.file_tree_split = false;
                        app.file_tree_scroll = 0;
                        return;
                    }
                }
                app.navigate_back();
            }
        }

        // Navigation
        Action::NavDown => match app.screen {
            Screen::PrList => app.nav_down(),
            Screen::PrDetail => {
                // j/k always drives the diff cursor regardless of selected pane
                let total = app.diff_lines_cache.as_ref().map(|l| l.len()).unwrap_or(0);
                if total > 0 && app.diff_cursor + 1 < total {
                    app.diff_cursor += 1;
                    let vh = app.diff_viewport_height.max(1);
                    if app.diff_cursor >= app.diff_scroll + vh {
                        app.diff_scroll = app.diff_cursor.saturating_sub(vh - 1);
                    }
                }
            }
            Screen::DoubleCheck => {
                if app.double_check_pane == 0 {
                    let total = ui::screens::double_check::visible_comment_count(app);
                    if app.double_check_selected < total.saturating_sub(1) {
                        app.double_check_selected += 1;
                    }
                } else {
                    app.double_check_detail_scroll += 1;
                }
            }
            Screen::FileTree => {
                if app.file_tree_pane == 0 {
                    let max = app
                        .draft
                        .as_ref()
                        .map(|d| d.file_checklist.len().saturating_sub(1))
                        .unwrap_or(0);
                    if app.file_tree_line < max {
                        app.file_tree_line += 1;
                    }
                } else {
                    app.file_tree_scroll += 1;
                }
            }
            Screen::AgentConfig => {
                app.agent_config_selected =
                    (app.agent_config_selected + 1).min(app.agents.len().saturating_sub(1));
            }
            Screen::AiFixOutput => {
                app.fix_task_selected =
                    (app.fix_task_selected + 1).min(app.fix_tasks.len().saturating_sub(1));
                app.ai_fix_scroll = 0;
            }
            _ => {}
        },
        Action::NavUp => match app.screen {
            Screen::PrList => app.nav_up(),
            Screen::PrDetail => {
                // j/k always drives the diff cursor regardless of selected pane
                if app.diff_cursor > 0 {
                    app.diff_cursor -= 1;
                    if app.diff_cursor < app.diff_scroll {
                        app.diff_scroll = app.diff_cursor;
                    }
                }
            }
            Screen::DoubleCheck => {
                if app.double_check_pane == 0 {
                    app.double_check_selected = app.double_check_selected.saturating_sub(1);
                } else {
                    app.double_check_detail_scroll =
                        app.double_check_detail_scroll.saturating_sub(1);
                }
            }
            Screen::FileTree => {
                if app.file_tree_pane == 0 {
                    app.file_tree_line = app.file_tree_line.saturating_sub(1);
                } else {
                    app.file_tree_scroll = app.file_tree_scroll.saturating_sub(1);
                }
            }
            Screen::AgentConfig => {
                app.agent_config_selected = app.agent_config_selected.saturating_sub(1);
            }
            Screen::AiFixOutput => {
                app.fix_task_selected = app.fix_task_selected.saturating_sub(1);
                app.ai_fix_scroll = 0;
            }
            _ => {}
        },

        Action::NavRight => match app.screen {
            Screen::FileTree => {
                app.file_tree_pane = 1;
            }
            _ => {}
        },

        Action::NavLeft => match app.screen {
            Screen::FileTree => {
                if app.file_tree_fullscreen {
                    // Exit fullscreen first; stay in pane 1
                    app.file_tree_fullscreen = false;
                    app.file_tree_split = false;
                    app.file_tree_scroll = 0;
                } else {
                    app.file_tree_pane = 0;
                }
            }
            _ => {}
        },

        Action::NextPane => match app.screen {
            Screen::PrDetail => app.selected_pane = (app.selected_pane + 1) % 3,
            Screen::DoubleCheck => app.double_check_pane = (app.double_check_pane + 1) % 2,
            Screen::FileTree => app.file_tree_pane = (app.file_tree_pane + 1) % 2,
            Screen::AgentWizard => {
                app.wizard_field = match app.wizard_field {
                    crate::app::AgentWizardField::Id => crate::app::AgentWizardField::Name,
                    crate::app::AgentWizardField::Name => crate::app::AgentWizardField::Icon,
                    crate::app::AgentWizardField::Icon => {
                        crate::app::AgentWizardField::SystemPrompt
                    }
                    crate::app::AgentWizardField::SystemPrompt => crate::app::AgentWizardField::Id,
                };
            }
            _ => {}
        },

        Action::ToggleFullscreen => {
            if app.screen == Screen::PrDetail {
                app.diff_fullscreen = !app.diff_fullscreen;
                if !app.diff_fullscreen {
                    app.diff_split_mode = false;
                }
                app.diff_scroll = 0;
            } else if app.screen == Screen::FileTree && app.file_tree_pane == 1 {
                app.file_tree_fullscreen = !app.file_tree_fullscreen;
                if !app.file_tree_fullscreen {
                    app.file_tree_split = false;
                }
                app.file_tree_scroll = 0;
            } else if app.screen == Screen::AiFixOutput {
                app.ai_fix_fullscreen = !app.ai_fix_fullscreen;
            }
        }
        Action::ToggleSplitDiff => {
            if app.screen == Screen::PrDetail {
                app.diff_split_mode = !app.diff_split_mode;
                if app.diff_split_mode {
                    app.diff_fullscreen = true;
                }
                app.diff_scroll = 0;
            } else if app.screen == Screen::FileTree && app.file_tree_pane == 1 {
                app.file_tree_split = !app.file_tree_split;
                if app.file_tree_split {
                    app.file_tree_fullscreen = true;
                }
                app.file_tree_scroll = 0;
            }
        }

        // Review Actions
        Action::GenerateReview => {
            if app.screen == Screen::PrDetail {
                if has_existing_local_review(app) {
                    // Already have unpublished AI/manual comments — go straight to DoubleCheck
                    // so the user can manage them. From there, [R] re-runs the agents if needed.
                    if let (Some(draft), Some(diff)) = (&mut app.draft, &app.current_diff.clone()) {
                        populate_checklist_from_diff(draft, diff);
                    }
                    app.navigate_to(Screen::DoubleCheck);
                } else {
                    start_agent_runner(app, config);
                }
            }
        }
        Action::ManualComment => {
            if app.screen == Screen::PrDetail || app.screen == Screen::FileTree {
                app.compose_quick_mode = false;
                app.compose_editor = crate::ui::editor::PrismEditor::new(String::new());
                app.editing_comment_id = None;
                // When in the diff pane, pre-fill file/line from cursor position
                if app.screen == Screen::PrDetail && app.selected_pane == 1 {
                    if let Some(lines) = &app.diff_lines_cache {
                        let (file, line) = diff_cursor_location(lines, app.diff_cursor);
                        app.compose_file_path = file;
                        app.compose_line = line;
                    }
                } else {
                    app.compose_file_path = None;
                    app.compose_line = None;
                }
                app.navigate_to(Screen::ReviewCompose);
            } else if app.screen == Screen::DoubleCheck {
                // [c] on DoubleCheck opens new comment (not edit)
                app.compose_quick_mode = false;
                app.compose_editor = crate::ui::editor::PrismEditor::new(String::new());
                app.editing_comment_id = None;
                app.navigate_to(Screen::ReviewCompose);
            }
        }
        Action::GenerateBody => {
            // [g] in DoubleCheck — edit selected comment
            if app.screen == Screen::DoubleCheck {
                if let Some((_, comment)) =
                    ui::screens::double_check::comment_at(app, app.double_check_selected)
                {
                    let body = comment.effective_body().to_string();
                    let id = comment.id;
                    app.compose_editor = crate::ui::editor::PrismEditor::new(body);
                    app.editing_comment_id = Some(id);
                    app.compose_quick_mode = false;
                    app.navigate_to(Screen::ReviewCompose);
                }
            }
            // [g] in SummaryPreview — LLM-generate the review body
            if app.screen == Screen::SummaryPreview && !app.review_body_generating {
                let comments: Vec<String> = app.draft.as_ref()
                    .map(|d| d.submittable_comments()
                        .iter()
                        .map(|c| {
                            let file = c.file_path.as_deref().unwrap_or("(general)");
                            let line = c.line.map(|l| format!(":{l}")).unwrap_or_default();
                            format!("[{}] {}{} — {}", c.severity, file, line, c.effective_body())
                        })
                        .collect())
                    .unwrap_or_default();

                if comments.is_empty() {
                    app.show_info("No comments", "Approve some comments first before generating a review body.");
                } else {
                    let pr_title = app.current_pr.as_ref().map(|p| p.title.clone()).unwrap_or_default();
                    let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
                    app.review_body_generating = true;

                    let tx = event_tx.clone();
                    let llm = config.llm.clone();
                    let client = reqwest::Client::new();
                    tokio::spawn(async move {
                        let system = "You are a senior code reviewer writing the top-level summary of a GitHub pull request review. \
                            Be concise, professional, and constructive. \
                            Write plain text (no markdown fences). \
                            Focus on the overall quality, patterns found, and the most important issues.";

                        let comment_list = comments.join("\n");
                        let prompt = format!(
                            "PR #{pr_num}: {pr_title}\n\n\
                            The following inline comments were found during review:\n\
                            {comment_list}\n\n\
                            Write a concise review summary (2-4 paragraphs) that: \
                            1) States the overall quality and what the PR does, \
                            2) Highlights the most critical issues found, \
                            3) Mentions any positive aspects, \
                            4) Gives a clear recommendation (approve / request changes)."
                        );

                        let model = llm.model.clone();
                        let result = agents::runner::call_provider(
                            &client, &llm, &model,
                            0.3, 1024,
                            system, &prompt,
                            60, "review-body-gen",
                        ).await;

                        match result {
                            Ok(body) => { let _ = tx.send(AppEvent::ReviewBodyGenerated(body)); }
                            Err(e)   => { let _ = tx.send(AppEvent::ReviewBodyFailed(e.to_string())); }
                        }
                    });
                }
            }
        }
        Action::RestartReview => {
            if app.screen == Screen::AiFixOutput {
                // [R] from AiFixOutput: force re-run the fixes
                start_ai_fix(app, event_tx, config);
                return;
            }
            // [R] from DoubleCheck or PrDetail — confirm before discarding local AI comments
            // and re-running all agents.
            if app.screen == Screen::DoubleCheck || app.screen == Screen::PrDetail {
                app.popup = Some(PopupState {
                    title: "Restart Review".to_string(),
                    message: "Discard all local AI comments and re-run the agents?\n\nExisting GitHub comments are preserved.\n[Enter] Restart  |  [Esc] Cancel".to_string(),
                    kind: PopupKind::ConfirmRestart,
                });
            }
        }
        Action::OpenDoubleCheck => {
            if let Some(pr) = &app.current_pr {
                let pr_num = pr.number;
                app.draft.get_or_insert_with(|| {
                    review::models::ReviewDraft::new(pr_num, review::models::ReviewMode::ManualOnly)
                });
                if let (Some(draft), Some(diff)) = (&mut app.draft, &app.current_diff.clone()) {
                    populate_checklist_from_diff(draft, diff);
                }
                app.navigate_to(Screen::DoubleCheck);
            }
        }

        Action::AgentWizard => {
            app.wizard_id.clear();
            app.wizard_name.clear();
            app.wizard_icon = "🤖".to_string();
            app.wizard_prompt_editor = crate::ui::editor::PrismEditor::new(String::new());
            app.wizard_field = crate::app::AgentWizardField::Id;
            app.navigate_to(Screen::AgentWizard);
        }

        Action::Char(c) => {
            if app.screen == Screen::AgentWizard {
                match app.wizard_field {
                    crate::app::AgentWizardField::Id => app.wizard_id.push(c),
                    crate::app::AgentWizardField::Name => app.wizard_name.push(c),
                    crate::app::AgentWizardField::Icon => {
                        app.wizard_icon.clear();
                        app.wizard_icon.push(c);
                    }
                    crate::app::AgentWizardField::SystemPrompt => {}
                }
            }
        }
        Action::Delete => {
            if app.screen == Screen::AgentWizard {
                match app.wizard_field {
                    crate::app::AgentWizardField::Id => {
                        app.wizard_id.pop();
                    }
                    crate::app::AgentWizardField::Name => {
                        app.wizard_name.pop();
                    }
                    crate::app::AgentWizardField::Icon => {
                        app.wizard_icon.pop();
                    }
                    crate::app::AgentWizardField::SystemPrompt => {}
                }
            } else if app.screen == Screen::DoubleCheck {
                // [Del] — confirm before deleting selected comment
                if let Some((_, comment)) =
                    ui::screens::double_check::comment_at(app, app.double_check_selected)
                {
                    let local_id = comment.id;
                    let has_github = comment.github_id.is_some();
                    app.pending_delete_comment = Some(local_id);
                    let msg = if has_github {
                        "Remove this comment locally AND delete it from GitHub?".to_string()
                    } else {
                        "Remove this comment from the review?".to_string()
                    };
                    app.popup = Some(PopupState {
                        title: "Delete comment".to_string(),
                        message: msg,
                        kind: PopupKind::ConfirmDeleteComment,
                    });
                }
            }
        }
        Action::EnterInsert => {
            app.input_mode = InputMode::Insert;
        }
        Action::ExitInsert => {
            app.input_mode = InputMode::Normal;
        }

        Action::Confirm => match app.screen {
            Screen::PrList => {
                open_pr(app, event_tx, config).await;
            }
            Screen::ReviewCompose => {
                save_or_update_comment(app, event_tx, config).await;
                save_draft(app, config);
                app.navigate_back();
            }
            Screen::AgentWizard => {
                if app.wizard_id.is_empty() {
                    app.show_error("ID is required");
                } else {
                    let _ = save_agent_to_disk(app);
                    app.agents = agents::loader::load_agents(config).unwrap_or_default();
                    app.navigate_back();
                }
            }
            _ => {}
        },

        Action::ToggleItem => {
            if app.screen == Screen::DoubleCheck {
                toggle_comment(app);
                save_draft(app, config);
            } else if app.screen == Screen::AgentConfig {
                if let Some(agent) = app.agents.get_mut(app.agent_config_selected) {
                    agent.agent.enabled = !agent.agent.enabled;
                }
            }
        }

        Action::SelectAll => {
            if app.screen == Screen::DoubleCheck {
                if let Some(draft) = &mut app.draft {
                    for c in &mut draft.comments {
                        // Skip already-published GitHub comments — their status is irrelevant
                        if c.github_id.is_some() { continue; }
                        if c.status != review::models::CommentStatus::Approved {
                            c.status = review::models::CommentStatus::Approved;
                        }
                    }
                }
            }
        }
        Action::DeselectAll => {
            if app.screen == Screen::DoubleCheck {
                if let Some(draft) = &mut app.draft {
                    for c in &mut draft.comments {
                        // Skip already-published GitHub comments — use [Del/-] to remove them
                        if c.github_id.is_some() { continue; }
                        c.status = review::models::CommentStatus::Rejected;
                    }
                }
            }
        }
        Action::PreviewSummary => {
            if app.screen == Screen::DoubleCheck {
                app.navigate_to(Screen::SummaryPreview);
            }
        }
        Action::Publish => {
            if app.screen == Screen::SummaryPreview {
                app.pending_publish = Some(crate::app::PendingPublish::FullReview);
                app.popup = Some(PopupState {
                    title: "Publish Review".to_string(),
                    message: "Submit all approved comments to GitHub?".to_string(),
                    kind: PopupKind::ConfirmPublish,
                });
            }
        }

        Action::CheckFile => {
            if app.screen == Screen::FileTree {
                if let Some(draft) = &mut app.draft {
                    if let Some((_path, checked)) =
                        draft.file_checklist.iter_mut().nth(app.file_tree_line)
                    {
                        *checked = !*checked;
                    }
                }
                save_draft(app, config);
            }
        }

        Action::InsertSuggestion => {
            if app.screen == Screen::ReviewCompose {
                let template = "```suggestion\n\n```".to_string();
                app.compose_editor = crate::ui::editor::PrismEditor::new(template);
                app.compose_editor.is_insert_mode = true;
                app.input_mode = InputMode::Insert;
            }
        }

        Action::ScrollDown => {
            if app.screen == Screen::PrDetail {
                let total = app.diff_lines_cache.as_ref().map(|l| l.len()).unwrap_or(0);
                let step = (app.diff_viewport_height / 2).max(1);
                app.diff_cursor = (app.diff_cursor + step).min(total.saturating_sub(1));
                let vh = app.diff_viewport_height.max(1);
                if app.diff_cursor >= app.diff_scroll + vh {
                    app.diff_scroll = app.diff_cursor.saturating_sub(vh - 1);
                }
            } else if app.screen == Screen::AiFixOutput {
                app.ai_fix_scroll = app.ai_fix_scroll.saturating_add(3);
            }
        }
        Action::ScrollUp => {
            if app.screen == Screen::PrDetail {
                let step = (app.diff_viewport_height / 2).max(1);
                app.diff_cursor = app.diff_cursor.saturating_sub(step);
                if app.diff_cursor < app.diff_scroll {
                    app.diff_scroll = app.diff_cursor;
                }
            } else if app.screen == Screen::AiFixOutput {
                app.ai_fix_scroll = app.ai_fix_scroll.saturating_sub(3);
            }
        }
        Action::PageDown => {
            if app.screen == Screen::PrDetail {
                let total = app.diff_lines_cache.as_ref().map(|l| l.len()).unwrap_or(0);
                let step = app.diff_viewport_height.saturating_sub(1).max(1);
                app.diff_cursor = (app.diff_cursor + step).min(total.saturating_sub(1));
                let vh = app.diff_viewport_height.max(1);
                if app.diff_cursor >= app.diff_scroll + vh {
                    app.diff_scroll = app.diff_cursor.saturating_sub(vh - 1);
                }
            }
        }
        Action::PageUp => {
            if app.screen == Screen::PrDetail {
                let step = app.diff_viewport_height.saturating_sub(1).max(1);
                app.diff_cursor = app.diff_cursor.saturating_sub(step);
                if app.diff_cursor < app.diff_scroll {
                    app.diff_scroll = app.diff_cursor;
                }
            }
        }
        Action::Refresh => {
            match app.screen {
                Screen::PrList => {
                    app.set_status("Refreshing PR list…");
                    app.pr_list_loading = true;
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
                Screen::PrDetail => {
                    if let Some(pr_num) = app.current_pr.as_ref().map(|p| p.number) {
                        app.set_status("Refreshing PR…");
                        app.diff_loading = true;
                        // Reload diff + reviews
                        let tx = event_tx.clone();
                        let cfg = config.clone();
                        tokio::spawn(async move {
                            match load_pr_diff(&cfg, pr_num).await {
                                Ok(diff) => {
                                    let _ = tx.send(AppEvent::DiffLoaded(diff));
                                }
                                Err(e) => {
                                    let _ =
                                        tx.send(AppEvent::Error(format!("Refresh failed: {e}")));
                                }
                            }
                        });
                        let tx = event_tx.clone();
                        let cfg = config.clone();
                        tokio::spawn(async move {
                            if let Ok(api) = make_github_api(&cfg).await {
                                let reviews = api.list_reviews(pr_num).await.unwrap_or_default();
                                let comments =
                                    api.list_inline_comments(pr_num).await.unwrap_or_default();
                                let _ = tx.send(AppEvent::ReviewsLoaded(pr_num, reviews, comments));
                            }
                        });
                    }
                }
                _ => {}
            }
        }
        Action::ShowStats => {
            app.show_stats = !app.show_stats;
        }
        Action::Settings => {
            app.navigate_to(Screen::Settings);
        }
        Action::AgentConfig => {
            app.navigate_to(Screen::AgentConfig);
        }
        Action::Help => {
            app.show_help = !app.show_help;
        }
        Action::FileTree => {
            if app.current_pr.is_some() {
                app.navigate_to(Screen::FileTree);
            }
        }
        Action::OpenBrowser => {
            if let Some(pr) = &app.current_pr {
                let _ = std::process::Command::new("xdg-open")
                    .arg(&pr.html_url)
                    .spawn();
            }
        }
        Action::AiFix => {
            if app.screen == Screen::DoubleCheck || app.screen == Screen::AiFixOutput {
                let current_pr_num = app.current_pr.as_ref().map(|p| p.number);
                let cached_same_pr = app.fix_tasks_pr == current_pr_num
                    && !app.fix_tasks.is_empty()
                    && !app.ai_fix_loading;
                if cached_same_pr && app.screen == Screen::DoubleCheck {
                    // Reuse existing results — just navigate back
                    app.navigate_to(Screen::AiFixOutput);
                } else if app.screen == Screen::DoubleCheck {
                    start_ai_fix(app, event_tx, config);
                }
            }
        }
        Action::ApplyFix => {
            if app.screen == Screen::AiFixOutput {
                apply_fix(app);
            }
        }
        Action::CopyOutput => {
            if app.screen == Screen::AiFixOutput {
                copy_fix_output_to_clipboard(app);
            }
        }
        _ => {}
    }
}

/// Extract (file_path, new_file_line_number) from a position in the unified diff line cache.
/// Used to pre-fill the file/line when opening a manual comment from the diff pane.
fn diff_cursor_location(lines: &[String], cursor: usize) -> (Option<String>, Option<u32>) {
    let mut file: Option<String> = None;
    let mut new_line: u32 = 1;

    for (i, raw) in lines.iter().enumerate() {
        if raw.starts_with("+++ ") {
            let path = raw
                .trim_start_matches("+++ b/")
                .trim_start_matches("+++ a/")
                .trim_start_matches("+++ ");
            file = Some(path.to_string());
            new_line = 1;
        } else if raw.starts_with("@@") {
            if let Some(after) = raw.strip_prefix("@@ -") {
                if let Some((_old, rest)) = after.split_once(' ') {
                    if let Some(new_part) = rest.strip_prefix('+') {
                        let new_start: u32 = new_part
                            .split_whitespace()
                            .next()
                            .unwrap_or("1")
                            .split(',')
                            .next()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(1);
                        new_line = new_start;
                    }
                }
            }
            if i == cursor {
                return (file, None);
            }
        } else if raw.starts_with('+') {
            if i == cursor {
                return (file, Some(new_line));
            }
            new_line += 1;
        } else if raw.starts_with('-') {
            if i == cursor {
                return (file, Some(new_line));
            }
            // removed line: don't advance new_line
        } else if raw.starts_with(' ') {
            if i == cursor {
                return (file, Some(new_line));
            }
            new_line += 1;
        } else if i == cursor {
            return (file, None);
        }
    }
    (file, None)
}

/// Returns true only if there are locally-generated (AI or manual) comments that
/// haven't been published yet. GitHub-imported comments (github_id present) don't
/// count — they are always preserved when agents re-run.
fn has_existing_local_review(app: &App) -> bool {
    let pr_num = app.current_pr.as_ref().map(|p| p.number).unwrap_or(0);
    app.draft
        .as_ref()
        .map(|d| d.pr_number == pr_num && d.comments.iter().any(|c| c.github_id.is_none()))
        .unwrap_or(false)
}

fn populate_checklist_from_diff(draft: &mut review::models::ReviewDraft, diff: &str) {
    if draft.file_checklist.is_empty() {
        for line in diff.lines() {
            if let Some(rest) = line.strip_prefix("diff --git ") {
                if let Some(b_part) = rest.split(" b/").nth(1) {
                    let path = b_part.trim().to_string();
                    if !path.is_empty() {
                        draft.file_checklist.insert(path, false);
                    }
                }
            }
        }
    }
}

fn save_draft(app: &App, config: &config::AppConfig) {
    if let Some(draft) = &app.draft {
        let repo_slug = format!("{}/{}", config.github.owner, config.github.repo);
        review::draft_store::save(draft, &repo_slug);
    }
}

fn prune_closed_drafts(repo_slug: &str, open_numbers: &[u64]) {
    review::draft_store::prune_closed(repo_slug, open_numbers);
}

/// Build a per-line file-extension vector from a unified diff.
/// Each entry is `Some("rs")` for lines belonging to a `+++ b/foo.rs` file,
/// or `None` for lines with no recognised extension (meta/hunk headers).
fn build_diff_line_ext(diff: &str) -> Vec<Option<String>> {
    let mut result = Vec::new();
    let mut current_ext: Option<String> = None;
    for line in diff.lines() {
        if line.starts_with("+++ b/") {
            let path = &line[6..];
            current_ext = std::path::Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase());
        }
        result.push(current_ext.clone());
    }
    result
}

fn handle_agent_update(
    app: &mut App,
    update: agents::orchestrator::AgentUpdate,
    config: &config::AppConfig,
) {
    use agents::models::AgentStatus;
    if let AgentStatus::Done {
        input_tokens,
        output_tokens,
        ..
    } = &update.status
    {
        let model = app.config.llm.model.clone();
        let now = chrono::Utc::now();
        let stats = app.model_stats.entry(model).or_default();
        stats.calls += 1;
        stats.input_tokens += *input_tokens;
        stats.output_tokens += *output_tokens;
        if stats.start_date.is_none() {
            stats.start_date = Some(now);
        }
        let day_key = now.format("%Y-%m-%d").to_string();
        let day = stats.daily.entry(day_key).or_default();
        day.calls += 1;
        day.input_tokens += *input_tokens;
        day.output_tokens += *output_tokens;
    }
    app.agent_statuses.insert(update.agent_id, update.status);

    if app.screen == Screen::AgentRunner {
        let all_done = app.agents.iter().filter(|a| a.agent.enabled).all(|a| {
            matches!(
                app.agent_statuses.get(&a.agent.id),
                Some(AgentStatus::Done { .. })
                    | Some(AgentStatus::Failed { .. })
                    | Some(AgentStatus::Skipped { .. })
            )
        });
        if all_done && !app.agents_committed {
            app.agents_committed = true;
            // Clear abort handle — agents finished naturally, no longer "running"
            app.agent_abort = None;
            if let Some(draft) = &mut app.draft {
                for status in app.agent_statuses.values() {
                    if let AgentStatus::Done { comments, .. } = status {
                        for c in comments {
                            draft.add_comment(c.clone());
                        }
                    }
                }
            }
            save_draft(app, config);
            app.navigate_to(Screen::DoubleCheck);
        }
    }
}

fn handle_sequence(app: &mut App, seq: KeySequence) {
    match seq {
        KeySequence::GoTop => app.go_top(),
        _ => {}
    }
}

async fn make_github_api(config: &config::AppConfig) -> Result<github::api::GitHubApi> {
    let client = github::client::GitHubClient::new(
        &config.github.token,
        &config.github.owner,
        &config.github.repo,
    )?;
    Ok(github::api::GitHubApi::new(client))
}

async fn load_pr_list(config: &config::AppConfig) -> Result<Vec<github::models::PrSummary>> {
    let api = make_github_api(config).await?;
    api.list_prs(config.github.per_page).await
}

async fn load_pr_details(
    config: &config::AppConfig,
    pr_num: u64,
) -> Result<github::models::PrDetails> {
    let api = make_github_api(config).await?;
    api.get_pr_details(pr_num).await
}

async fn load_pr_diff(config: &config::AppConfig, pr_num: u64) -> Result<String> {
    let api = make_github_api(config).await?;
    api.get_pr_diff(pr_num).await
}

async fn open_pr(
    app: &mut App,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    config: &config::AppConfig,
) {
    let pr_num = match app.selected_pr() {
        Some(pr) => pr.number,
        None => return,
    };

    // Clear all state from the previously viewed PR before loading the new one
    app.current_pr = None;
    app.current_diff = None;
    app.diff_lines_cache = None;
    app.diff_line_ext = Vec::new();
    // Pre-create a blank draft so the file checklist is populated when the diff arrives,
    // even for clean PRs that have no GitHub reviews/comments.
    app.draft = Some(review::models::ReviewDraft::new(
        pr_num,
        review::models::ReviewMode::ManualOnly,
    ));
    app.agent_statuses.clear();
    app.diff_scroll = 0;
    app.diff_cursor = 0;
    app.description_scroll = 0;
    app.compose_file_path = None;
    app.compose_line = None;
    app.compose_context = Vec::new();
    app.current_ticket = None;
    app.project_conventions = None;
    app.pr_description_md_cache = None;
    app.split_diff_cache = None;
    app.file_tree_line = 0;
    app.file_tree_pane = 0;
    app.file_tree_scroll = 0;
    app.file_tree_fullscreen = false;
    app.file_tree_split = false;

    app.pr_loading = true;
    app.diff_loading = true;
    app.navigate_to(Screen::PrDetail);

    let tx = event_tx.clone();
    let cfg = config.clone();
    tokio::spawn(async move {
        match load_pr_details(&cfg, pr_num).await {
            Ok(pr) => {
                let _ = tx.send(AppEvent::PrLoaded(Box::new(pr)));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Error(e.to_string()));
            }
        }
    });

    let tx = event_tx.clone();
    let cfg = config.clone();
    tokio::spawn(async move {
        match load_pr_diff(&cfg, pr_num).await {
            Ok(diff) => {
                let _ = tx.send(AppEvent::DiffLoaded(diff));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Error(e.to_string()));
            }
        }
    });

    // Load existing reviews and inline comments — carries pr_num so stale results can be discarded
    let tx = event_tx.clone();
    let cfg = config.clone();
    tokio::spawn(async move {
        if let Ok(api) = make_github_api(&cfg).await {
            let reviews = api.list_reviews(pr_num).await.unwrap_or_default();
            let comments = api.list_inline_comments(pr_num).await.unwrap_or_default();
            let _ = tx.send(AppEvent::ReviewsLoaded(pr_num, reviews, comments));
        }
    });

    // Opportunistically fetch project conventions — CONTRIBUTING.md or PR template
    let tx = event_tx.clone();
    let cfg = config.clone();
    tokio::spawn(async move {
        if let Ok(api) = make_github_api(&cfg).await {
            let conventions = match api.get_file_content("CONTRIBUTING.md").await {
                Some(c) => Some(c),
                None => {
                    api.get_file_content(".github/PULL_REQUEST_TEMPLATE.md")
                        .await
                }
            };
            let _ = tx.send(AppEvent::ConventionsLoaded(conventions));
        }
    });
}

fn start_agent_runner(app: &mut App, config: &config::AppConfig) {
    use agents::context::ReviewContext;
    use agents::orchestrator::Orchestrator;
    let pr = match &app.current_pr {
        Some(p) => p.clone(),
        None => return,
    };
    let diff = app.current_diff.clone().unwrap_or_default();
    let repo_slug = format!("{}/{}", config.github.owner, config.github.repo);
    let mut ctx = ReviewContext::from_pr(&pr, &diff, None, &repo_slug);
    ctx.project_conventions = app.project_conventions.clone();

    // Preserve comments that already exist on GitHub (loaded from ReviewsLoaded).
    // Only clear locally-generated AI/manual comments so the user starts fresh
    // without losing the existing GitHub thread context.
    let github_comments: Vec<_> = app.draft.as_ref()
        .map(|d| d.comments.iter().filter(|c| c.github_id.is_some()).cloned().collect())
        .unwrap_or_default();

    let mut new_draft = review::models::ReviewDraft::new(
        pr.number,
        review::models::ReviewMode::AiOnly,
    );
    for c in github_comments {
        new_draft.comments.push(c);
    }
    app.draft = Some(new_draft);

    if let (Some(draft), Some(diff)) = (&mut app.draft, &app.current_diff) {
        populate_checklist_from_diff(draft, diff);
    }
    app.agent_statuses.clear();
    app.agents_committed = false;
    app.navigate_to(Screen::AgentRunner);

    let orchestrator = Orchestrator::new(config.clone());
    let (rx, abort) = orchestrator.run_all(app.agents.clone(), ctx);
    app.agent_rx = Some(rx);
    app.agent_abort = Some(abort);
}

async fn save_or_update_comment(
    app: &mut App,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    config: &config::AppConfig,
) {
    use review::models::{CommentSource, GeneratedComment, Severity};
    let text = app.compose_editor.get_text();
    if text.trim().is_empty() {
        return;
    }

    if let Some(edit_id) = app.editing_comment_id.take() {
        // Edit mode — update existing comment
        let (github_id, is_review_summary) = app
            .draft
            .as_ref()
            .and_then(|d| d.comments.iter().find(|c| c.id == edit_id))
            .map(|c| {
                (
                    c.github_id,
                    matches!(c.source, CommentSource::GithubReview { .. }),
                )
            })
            .unwrap_or((None, false));

        if let Some(draft) = &mut app.draft {
            if let Some(comment) = draft.comments.iter_mut().find(|c| c.id == edit_id) {
                comment.edited_body = Some(text.clone());
            }
        }

        // Sync to GitHub if this came from GitHub
        if let (Some(gh_id), Some(pr)) = (github_id, app.current_pr.as_ref().map(|p| p.number)) {
            let tx = event_tx.clone();
            let cfg = config.clone();
            let local_id = edit_id;
            let new_body = text.clone();
            tokio::spawn(async move {
                if let Ok(api) = make_github_api(&cfg).await {
                    let result = if is_review_summary {
                        api.update_review(pr, gh_id, &new_body).await
                    } else {
                        api.update_review_comment(gh_id, &new_body).await
                    };
                    match result {
                        Ok(_) => {
                            let _ = tx.send(AppEvent::CommentUpdated(local_id, new_body));
                        }
                        Err(e) => {
                            let _ = tx.send(AppEvent::Error(format!("Update failed: {e}")));
                        }
                    }
                }
            });
        }
    } else {
        // Create mode — add new comment
        let comment = GeneratedComment::new(
            CommentSource::Manual,
            text,
            Severity::Suggestion,
            app.compose_file_path.clone(),
            app.compose_line,
        );
        if let Some(draft) = &mut app.draft {
            draft.add_comment(comment);
        }
    }
}

fn save_compose_comment(app: &mut App) {
    use review::models::{CommentSource, GeneratedComment, Severity};
    let text = app.compose_editor.get_text();
    if !text.trim().is_empty() {
        let comment = GeneratedComment::new(
            CommentSource::Manual,
            text,
            Severity::Suggestion,
            app.compose_file_path.clone(),
            app.compose_line,
        );
        if let Some(draft) = &mut app.draft {
            draft.add_comment(comment);
        }
    }
}

fn toggle_comment(app: &mut App) {
    use review::models::CommentStatus;
    // Resolve visual index → original draft index via threaded order
    let orig_idx =
        ui::screens::double_check::comment_at(app, app.double_check_selected).map(|(i, _)| i);
    if let (Some(idx), Some(draft)) = (orig_idx, &mut app.draft) {
        if let Some(comment) = draft.comments.get_mut(idx) {
            // GitHub comments are already published — toggling their local status
            // is meaningless (the publisher skips them anyway). Use [Del/-] to delete.
            if comment.github_id.is_some() {
                return;
            }
            comment.status = match comment.status {
                CommentStatus::Approved => CommentStatus::Rejected,
                _ => CommentStatus::Approved,
            };
        }
    }
}

async fn publish_review(
    app: &mut App,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    config: &config::AppConfig,
) {
    let draft = match &app.draft {
        Some(d) => d.clone(),
        None => return,
    };
    let tx = event_tx.clone();
    let cfg = config.clone();
    tokio::spawn(async move {
        let api = make_github_api(&cfg).await.unwrap();
        let publisher = review::publisher::ReviewPublisher::new(api);
        match publisher.publish(&draft).await {
            Ok(_) => {
                let _ = tx.send(AppEvent::PublishDone);
            }
            Err(e) => {
                let _ = tx.send(AppEvent::PublishFailed(e.to_string()));
            }
        }
    });
}

fn save_agent_to_disk(app: &App) -> Result<std::path::PathBuf> {
    // Use the configured agents_dir (expand leading ~)
    let agents_dir_str = &app.config.agents.agents_dir;
    let agents_dir = if agents_dir_str.starts_with('~') {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home).join(&agents_dir_str[2..])
    } else {
        std::path::PathBuf::from(agents_dir_str)
    };
    std::fs::create_dir_all(&agents_dir)?;
    let file_path = agents_dir.join(format!("{}.md", app.wizard_id));

    let prompt_text = app.wizard_prompt_editor.get_text();
    let prompt_suffix = "Please respond with a JSON array where each element has: \
        file_path (string or null), line (number or null), \
        severity (\"critical\"|\"warning\"|\"suggestion\"|\"praise\"), and body (string).";

    let content = format!(
        "---\n\
        id: {id}\n\
        name: {name}\n\
        description: Custom agent\n\
        enabled: true\n\
        order: 99\n\
        icon: \"{icon}\"\n\
        color: cyan\n\
        synthesis: false\n\
        context:\n\
        \x20 include_diff: true\n\
        \x20 include_pr_description: true\n\
        \x20 include_ticket: false\n\
        \x20 include_file_list: false\n\
        \x20 exclude_patterns: []\n\
        \x20 include_patterns: []\n\
        ---\n\n\
        ## System Prompt\n\n\
        {prompt}\n\n\
        ## Prompt Suffix\n\n\
        {suffix}\n",
        id = app.wizard_id,
        name = app.wizard_name,
        icon = app.wizard_icon,
        prompt = prompt_text,
        suffix = prompt_suffix,
    );
    std::fs::write(&file_path, content)?;
    Ok(file_path)
}

fn start_ai_fix(
    app: &mut App,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    config: &config::AppConfig,
) {
    use app::{FixTask, FixTaskStatus};

    // Collect approved/pending comments that have a file location — those are fixable
    let comments: Vec<_> = app.draft.as_ref()
        .map(|d| d.submittable_comments()
            .into_iter()
            .filter(|c| c.file_path.is_some() && c.github_id.is_none())
            .cloned()
            .collect())
        .unwrap_or_default();

    if comments.is_empty() {
        app.show_info("No fixable comments", "Approve some AI-generated inline comments first.\nComments need a file path to be auto-fixed.");
        return;
    }

    let diff = app.current_diff.clone().unwrap_or_default();
    let pr_title = app.current_pr.as_ref().map(|p| p.title.clone()).unwrap_or_default();

    // Build fix tasks
    app.fix_tasks = comments.iter().enumerate().map(|(i, c)| {
        let file = c.file_path.clone().unwrap_or_default();
        let line = c.line.unwrap_or(0);
        let body = c.effective_body().to_string();
        let severity = format!("{}", c.severity);

        let prompt = format!(
            "PR: {pr_title}\n\n\
            You are reviewing a code fix request.\n\
            File: {file} (line {line})\n\
            Issue [{severity}]: {body}\n\n\
            Based on the diff below, provide the exact fix for this issue.\n\
            Output the fix as a unified diff in a ```diff code block so it can be applied with `patch`.\n\
            Then briefly explain what was changed and why.\n\n\
            ```diff\n{diff}\n```",
            diff = if diff.len() > 6000 { &diff[..6000] } else { &diff }
        );

        FixTask {
            index: i,
            location: format!("{}:{}", file, line),
            source: format!("[{}]", severity),
            summary: body.chars().take(60).collect(),
            status: FixTaskStatus::Pending,
            output: String::new(),
            prompt,
            file_path: file.clone(),
        }
    }).collect();

    app.fix_task_selected = 0;
    app.ai_fix_scroll = 0;
    app.ai_fix_loading = true;
    app.ai_fix_fullscreen = false;
    app.fix_tasks_pr = app.current_pr.as_ref().map(|p| p.number);
    app.navigate_to(Screen::AiFixOutput);

    // Spawn one task per fix sequentially via channel
    let tx = event_tx.clone();
    let llm = config.llm.clone();
    let tasks: Vec<_> = app.fix_tasks.iter().map(|t| (t.index, t.prompt.clone())).collect();

    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let system = "You are a senior developer fixing code issues. \
            Output the fix as a unified diff inside a ```diff code block (so it can be applied with `patch`). \
            Then briefly explain the change. Be concise and practical.";

        for (idx, prompt) in tasks {
            let _ = tx.send(AppEvent::FixTaskChunk(idx, String::new())); // mark as Running

            let model = llm.model.clone();
            let result = agents::runner::call_provider(
                &client, &llm, &model,
                0.2, 1024,
                system, &prompt,
                300, "ai-fix",
            ).await;

            match result {
                Ok(response) => {
                    let _ = tx.send(AppEvent::FixTaskChunk(idx, response));
                    let _ = tx.send(AppEvent::FixTaskDone(idx));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FixTaskFailed(idx, e.to_string()));
                }
            }
        }
    });
}

/// Extract the first ```diff...``` block from text, or fall back to the first fenced block.
fn extract_diff_block(text: &str) -> Option<String> {
    // Try ```diff first
    if let Some(start) = text.find("```diff\n") {
        let rest = &text[start + 8..];
        if let Some(end) = rest.find("\n```") {
            return Some(rest[..end].to_string());
        }
    }
    // Fall back to any fenced block
    if let Some(start) = text.find("```\n") {
        let rest = &text[start + 4..];
        if let Some(end) = rest.find("\n```") {
            return Some(rest[..end].to_string());
        }
    }
    None
}

/// Apply the diff from the selected fix task using the system `patch` command.
/// Tries -p0, -p1, and -p2 strip levels to handle different diff header formats.
fn apply_fix(app: &mut App) {
    let task = match app.fix_tasks.get(app.fix_task_selected) {
        Some(t) => t.clone(),
        None => return,
    };

    if task.file_path.is_empty() {
        app.show_info("Cannot apply fix", "No file path associated with this task.");
        return;
    }

    let diff = match extract_diff_block(&task.output) {
        Some(d) => d,
        None => {
            app.show_info(
                "Cannot apply fix",
                "No ```diff block found in the LLM output.\n\
                 The model did not produce a unified diff.\n\
                 Use [y] to copy the suggestion and apply it manually.",
            );
            return;
        }
    };

    // Ensure the diff ends with a newline (patch requires it)
    let diff = if diff.ends_with('\n') { diff } else { format!("{diff}\n") };

    // Try strip levels 0, 1, 2 — LLMs use varying prefix styles
    for strip in ["0", "1", "2"] {
        let tmp = std::env::temp_dir().join(format!("prism_fix_{}.patch", task.index));
        if std::fs::write(&tmp, diff.as_bytes()).is_err() {
            continue;
        }
        let out = std::process::Command::new("patch")
            .args([&format!("-p{strip}"), "--forward", "--batch", "--input"])
            .arg(&tmp)
            .output();
        let _ = std::fs::remove_file(&tmp);

        match out {
            Ok(result) if result.status.success() => {
                let stdout = String::from_utf8_lossy(&result.stdout).to_string();
                if let Some(t) = app.fix_tasks.get_mut(app.fix_task_selected) {
                    t.output.push_str(&format!("\n\n✓ Fix applied (patch -p{strip}):\n{stdout}"));
                }
                app.show_info("Fix applied", &format!("patch -p{strip} succeeded:\n{stdout}"));
                return;
            }
            _ => continue,
        }
    }

    // All strip levels failed — show a diagnostic
    // Run once more with -p1 to capture the real error message
    let tmp = std::env::temp_dir().join(format!("prism_fix_{}_err.patch", task.index));
    let _ = std::fs::write(&tmp, diff.as_bytes());
    let err_out = std::process::Command::new("patch")
        .args(["-p1", "--dry-run", "--input"])
        .arg(&tmp)
        .output();
    let _ = std::fs::remove_file(&tmp);

    let detail = err_out
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stderr).to_string();
            if s.trim().is_empty() { String::from_utf8_lossy(&o.stdout).to_string() } else { s }
        })
        .unwrap_or_else(|e| e.to_string());

    app.show_info(
        "Apply failed",
        &format!(
            "Could not apply the diff automatically.\n\
             patch error: {detail}\n\n\
             Tip: Use [y] to copy the suggestion to clipboard\n\
             and apply it manually in your editor."
        ),
    );
}

/// Copy the current task's output to the system clipboard (xclip / xsel).
fn copy_fix_output_to_clipboard(app: &mut App) {
    let output = match app.fix_tasks.get(app.fix_task_selected) {
        Some(t) if !t.output.is_empty() => t.output.clone(),
        _ => {
            app.show_info("Nothing to copy", "The selected task has no output yet.");
            return;
        }
    };

    // Try xclip first, then xsel
    let copied = try_copy_clipboard("xclip", &["-selection", "clipboard"], &output)
        || try_copy_clipboard("xsel", &["--clipboard", "--input"], &output)
        || try_copy_clipboard("wl-copy", &[], &output);

    if copied {
        app.show_info("Copied", "Fix output copied to clipboard.");
    } else {
        app.show_info(
            "Copy failed",
            "Could not find xclip, xsel, or wl-copy.\n\
             Install one of them to enable clipboard support.",
        );
    }
}

fn try_copy_clipboard(cmd: &str, args: &[&str], text: &str) -> bool {
    use std::io::Write;
    let Ok(mut child) = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .spawn()
    else { return false; };
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(text.as_bytes());
    }
    child.wait().map(|s| s.success()).unwrap_or(false)
}

fn init_tracing() -> Option<()> {
    if let Ok(file) = std::fs::File::create("prism.log") {
        tracing_subscriber::fmt()
            .with_writer(std::sync::Mutex::new(file))
            .with_env_filter(EnvFilter::new("prism=info"))
            .init();
    }
    Some(())
}
