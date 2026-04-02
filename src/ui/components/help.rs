use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

use crate::app::{App, Screen};
use crate::ui::theme::Theme;

// ── Help content definitions ────────────────────────────────────────────────

struct HelpEntry {
    key: &'static str,
    desc: &'static str,
}

struct ScreenHelp {
    title: &'static str,
    icon: &'static str,
    overview: &'static str,
    actions: Vec<HelpEntry>,
    navigation: Vec<HelpEntry>,
    tips: Vec<&'static str>,
}

fn help_for_screen(screen: &Screen) -> ScreenHelp {
    match screen {
        Screen::PrList => ScreenHelp {
            title: "PR List",
            icon: "📋",
            overview: "Browse open pull requests for the configured repository. Select a PR to open it for review. You can filter by title, author, or PR number.",
            actions: vec![
                HelpEntry { key: "Enter", desc: "Open selected PR" },
                HelpEntry { key: "o",     desc: "Open PR in browser" },
                HelpEntry { key: "F5",    desc: "Refresh PR list" },
                HelpEntry { key: "/",     desc: "Search / filter PRs" },
                HelpEntry { key: "a",     desc: "Configure AI agents" },
                HelpEntry { key: "S",     desc: "Settings" },
                HelpEntry { key: "q",     desc: "Quit" },
            ],
            navigation: vec![
                HelpEntry { key: "j / ↓", desc: "Next PR" },
                HelpEntry { key: "k / ↑", desc: "Previous PR" },
                HelpEntry { key: "gg",    desc: "Go to top" },
                HelpEntry { key: "G",     desc: "Go to bottom" },
                HelpEntry { key: "Esc",   desc: "Clear filter" },
            ],
            tips: vec![
                "Filter is case-insensitive and matches title, author, and number",
                "PRs are sorted by most recently updated",
                "Draft PRs are marked with [draft] in the title column",
            ],
        },

        Screen::PrDetail => ScreenHelp {
            title: "PR Detail",
            icon: "🔍",
            overview: "Review a pull request. Three panels: Description (left), Diff (center), Ticket (right). Start an AI review, write a quick comment, or navigate to the file tree for inline comments.",
            actions: vec![
                HelpEntry { key: "r",     desc: "Start AI-only review" },
                HelpEntry { key: "H",     desc: "Hybrid review (AI + manual)" },
                HelpEntry { key: "c",     desc: "Quick comment (posts directly)" },
                HelpEntry { key: "v",     desc: "View review session (DoubleCheck)" },
                HelpEntry { key: "f",     desc: "File tree with inline comments" },
                HelpEntry { key: "o",     desc: "Open PR in browser" },
                HelpEntry { key: "z",     desc: "Toggle diff fullscreen" },
            ],
            navigation: vec![
                HelpEntry { key: "Tab",   desc: "Switch panel (Desc → Diff → Ticket)" },
                HelpEntry { key: "j / k", desc: "Scroll focused panel" },
                HelpEntry { key: "J / K", desc: "Fast scroll (10 lines)" },
                HelpEntry { key: "Ctrl+d/u", desc: "Scroll half page" },
                HelpEntry { key: "Ctrl+f/b", desc: "Page down / up" },
                HelpEntry { key: "gg / G",  desc: "Top / bottom" },
                HelpEntry { key: "Esc",   desc: "Back to PR list" },
            ],
            tips: vec![
                "[c] posts a quick comment to the PR conversation immediately",
                "[r] runs all enabled AI agents — check [a] to configure them",
                "[z] maximizes the diff panel — press again to restore",
                "Panel in focus has a blue border",
            ],
        },

        Screen::FileTree => ScreenHelp {
            title: "File Tree",
            icon: "🌳",
            overview: "Browse all changed files in the PR. Check files as reviewed, view per-file diffs with syntax highlighting, and add inline comments to specific lines.",
            actions: vec![
                HelpEntry { key: "Enter",  desc: "Jump to file in main diff" },
                HelpEntry { key: "→ / l",  desc: "Open file detail panel" },
                HelpEntry { key: "x",      desc: "Toggle file as reviewed ✓" },
                HelpEntry { key: "c",      desc: "Comment on selected line" },
            ],
            navigation: vec![
                HelpEntry { key: "j / k",  desc: "Navigate files (left panel)" },
                HelpEntry { key: "j / k",  desc: "Navigate diff lines (right panel)" },
                HelpEntry { key: "J / K",  desc: "Scroll detail view" },
                HelpEntry { key: "← / h",  desc: "Back to file list" },
                HelpEntry { key: "Esc",    desc: "Back to PR Detail" },
            ],
            tips: vec![
                "Progress bar at the bottom shows how many files you've reviewed",
                "Lines annotated with your comments show in the comments panel below the diff",
                "Use [x] to mark a file as reviewed — tracked for your reference only",
            ],
        },

        Screen::ReviewCompose => ScreenHelp {
            title: "Compose Comment",
            icon: "✍️",
            overview: "Write a comment or code suggestion. Two modes: Quick Comment (posts directly to PR conversation) and Inline Review Comment (saved to your review session for DoubleCheck).",
            actions: vec![
                HelpEntry { key: "i",      desc: "Enter INSERT mode (start typing)" },
                HelpEntry { key: "Enter",  desc: "Add to review / Publish comment" },
                HelpEntry { key: "s",      desc: "Insert code suggestion block" },
                HelpEntry { key: "v",      desc: "View review session" },
                HelpEntry { key: "Esc",    desc: "Cancel / back to Normal mode" },
            ],
            navigation: vec![
                HelpEntry { key: "Esc",    desc: "Exit INSERT → NORMAL mode" },
                HelpEntry { key: "Enter",  desc: "New line (in INSERT mode)" },
            ],
            tips: vec![
                "NORMAL mode: use [i] to start editing, [Esc] to stop",
                "Code suggestions use GitHub's ```suggestion blocks — edit the code inside",
                "Quick Comment (orange header) posts immediately after confirmation",
                "Inline comments are added to your review draft in DoubleCheck",
                "Auto-translate to English can be enabled in config [publishing]",
            ],
        },

        Screen::AgentRunner => ScreenHelp {
            title: "AI Agent Runner",
            icon: "🤖",
            overview: "AI agents are analyzing the PR diff. Each agent has a specialized focus. Wait for all agents to finish, then review their comments in DoubleCheck.",
            actions: vec![
                HelpEntry { key: "Esc",    desc: "Cancel and go back" },
            ],
            navigation: vec![
                HelpEntry { key: "j / k",  desc: "Scroll agent list" },
            ],
            tips: vec![
                "Agents run concurrently up to the configured concurrency limit",
                "Failed agents can be retried — check [a] to configure agents",
                "Configure agent prompts in ~/.config/prism/agents/",
                "Timeout per agent is configurable in [agents] config section",
            ],
        },

        Screen::DoubleCheck => ScreenHelp {
            title: "Double-Check",
            icon: "✅",
            overview: "Review all generated comments (AI and manual). Approve the ones you want to include, reject the rest. Filter by agent. Then preview and submit your review to GitHub.",
            actions: vec![
                HelpEntry { key: "Space",  desc: "Toggle approve / reject selected" },
                HelpEntry { key: "A",      desc: "Approve all comments" },
                HelpEntry { key: "D",      desc: "Reject all comments" },
                HelpEntry { key: "c",      desc: "Write a new manual comment" },
                HelpEntry { key: "P",      desc: "Go to Summary Preview" },
                HelpEntry { key: "v",      desc: "View / return to PR detail" },
                HelpEntry { key: "1-7",    desc: "Filter by agent number" },
                HelpEntry { key: "0",      desc: "Clear filter (show all)" },
            ],
            navigation: vec![
                HelpEntry { key: "j / k",  desc: "Navigate comment list" },
                HelpEntry { key: "gg / G", desc: "Top / bottom" },
                HelpEntry { key: "Esc",    desc: "Back to PR detail" },
            ],
            tips: vec![
                "Only approved (✓) comments will be submitted to GitHub",
                "Rejected (✗) comments are hidden but not deleted",
                "Filter by agent to quickly approve all from a specific reviewer",
                "Manual comments from [c] appear here as 'manual' source",
            ],
        },

        Screen::SummaryPreview => ScreenHelp {
            title: "Summary Preview",
            icon: "📤",
            overview: "Final step before publishing. Review the list of approved inline comments, optionally generate a review body, choose the review type, then submit to GitHub.",
            actions: vec![
                HelpEntry { key: "Enter / p", desc: "Submit review to GitHub" },
                HelpEntry { key: "g",      desc: "Generate review body from comments" },
                HelpEntry { key: "←→",     desc: "Change review type (Comment / Request Changes / Approve)" },
                HelpEntry { key: "Esc",    desc: "Back to DoubleCheck" },
            ],
            navigation: vec![
                HelpEntry { key: "Tab",    desc: "Switch panel (Body ↔ Comments)" },
                HelpEntry { key: "j / k",  desc: "Scroll focused panel" },
            ],
            tips: vec![
                "Leave review body empty to submit only inline comments (no summary)",
                "REQUEST_CHANGES is disabled on your own PRs (GitHub limitation)",
                "APPROVE merges your comments and approves the PR in one action",
                "[g] generates a formatted body from approved comments using your template",
            ],
        },

        Screen::AgentConfig => ScreenHelp {
            title: "Agent Configuration",
            icon: "⚙️",
            overview: "Enable or disable AI review agents. Each agent has a specific focus area. Disabled agents are skipped when running reviews.",
            actions: vec![
                HelpEntry { key: "Space",  desc: "Toggle agent enabled / disabled" },
                HelpEntry { key: "Enter",  desc: "Toggle agent enabled / disabled" },
                HelpEntry { key: "Esc",    desc: "Back" },
            ],
            navigation: vec![
                HelpEntry { key: "j / k",  desc: "Navigate agent list" },
            ],
            tips: vec![
                "Agent definitions are YAML files in ~/.config/prism/agents/",
                "Each agent can have a custom model, temperature, and prompt",
                "Concurrency limit is set in [agents] config section",
            ],
        },

        Screen::AgentWizard => ScreenHelp {
            title: "Agent Wizard",
            icon: "🪄",
            overview: "Create a custom AI review agent. Fill in the ID (slug), Name, Icon, and the System Prompt that defines the agent's persona and instructions.",
            actions: vec![
                HelpEntry { key: "Tab",    desc: "Next field" },
                HelpEntry { key: "i",      desc: "Enter INSERT mode (start typing)" },
                HelpEntry { key: "Enter",  desc: "Save Agent (when in Normal mode)" },
                HelpEntry { key: "Esc",    desc: "Back" },
            ],
            navigation: vec![
                HelpEntry { key: "Tab",    desc: "Cycle through fields" },
            ],
            tips: vec![
                "Agent ID must be unique and use snake_case (e.g., 'my_agent')",
                "The System Prompt is the most important part — be specific about what the AI should look for",
                "New agents are saved as Markdown files in ~/.config/prism/agents/",
            ],
        },

        Screen::Settings => ScreenHelp {
            title: "Settings",
            icon: "🛠️",
            overview: "View the current configuration loaded from environment variables and ~/.config/prism/config.toml. Edit the config file to change settings.",
            actions: vec![
                HelpEntry { key: "Esc",    desc: "Back" },
            ],
            navigation: vec![
                HelpEntry { key: "j / k",  desc: "Scroll settings list" },
            ],
            tips: vec![
                "Config file: ~/.config/prism/config.toml",
                "Environment variables override config file values",
                "GITHUB_TOKEN, GITHUB_OWNER, GITHUB_REPO — GitHub auth",
                "ANTHROPIC_API_KEY — enables LLM features",
                "publishing.auto_translate_to_english = true — auto-translate to English",
            ],
        },

        Screen::Setup => ScreenHelp {
            title: "Setup Wizard",
            icon: "🚀",
            overview: "Configure your GitHub credentials to get started. PRISM needs a GitHub token, owner (org or username), and repository name.",
            actions: vec![
                HelpEntry { key: "Enter",  desc: "Save and connect to GitHub" },
                HelpEntry { key: "Tab",    desc: "Next field" },
                HelpEntry { key: "i",      desc: "Edit current field" },
                HelpEntry { key: "Esc",    desc: "Cancel edit" },
            ],
            navigation: vec![
                HelpEntry { key: "Tab / j/k", desc: "Switch between fields" },
            ],
            tips: vec![
                "Token is auto-detected from the gh CLI if available",
                "Owner/Repo is auto-detected from git remote via gh CLI",
                "Token needs repo, read:user, and pull_request scopes",
                "You can also set GITHUB_TOKEN, GITHUB_OWNER, GITHUB_REPO env vars",
            ],
        },
        &Screen::ClaudeCodeOutput => ScreenHelp {
            title: "Claude Code Output",
            icon: "✦",
            overview: "Claude Code's analysis and suggested fixes for the review comments.",
            actions: vec![
                HelpEntry { key: "j / k",   desc: "Scroll down / up" },
                HelpEntry { key: "G",        desc: "Go to bottom" },
                HelpEntry { key: "gg",       desc: "Go to top" },
                HelpEntry { key: "Esc",      desc: "Back to Double-Check" },
            ],
            navigation: vec![],
            tips: vec![
                "Claude Code processes the approved review comments",
                "Review the suggested changes before applying them",
            ],
        },
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────

pub fn render_help(frame: &mut Frame, app: &App) {
    let t = Theme::current(&app.config.ui.theme);
    let area = frame.area();

    // Large centered overlay
    let popup_area = {
        let w = (area.width * 80 / 100).max(60).min(area.width);
        let h = (area.height * 88 / 100).max(20).min(area.height);
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        Rect { x, y, width: w, height: h }
    };

    frame.render_widget(Clear, popup_area);

    let help = help_for_screen(&app.screen);
    let title = format!(" {} PRISM Help — {} ", help.icon, help.title);

    let outer = Block::default()
        .title(title.as_str())
        .title_style(Style::default().fg(t.title).add_modifier(Modifier::BOLD))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Rgb(10, 10, 20)));

    let inner = outer.inner(popup_area);
    frame.render_widget(outer, popup_area);

    // Layout: overview (top) | body (middle, 2 cols) | tips (bottom)
    let overview_h = 3u16;
    let tips_h = (help.tips.len() as u16 + 2).min(inner.height / 4);
    let body_h = inner.height.saturating_sub(overview_h + tips_h + 1);

    let rows = Layout::vertical([
        Constraint::Length(overview_h),
        Constraint::Length(body_h),
        Constraint::Length(1),
        Constraint::Length(tips_h),
    ]).split(inner);

    // ── Overview ────────────────────────────────────────────────────────────
    let overview = Paragraph::new(help.overview)
        .style(Style::default().fg(Color::White))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(" 📖 Overview ")
                .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(60, 60, 80)))
                .style(Style::default().bg(Color::Rgb(10, 10, 20))),
        );
    frame.render_widget(overview, rows[0]);

    // ── Actions + Navigation side by side ───────────────────────────────────
    let cols = Layout::horizontal([
        Constraint::Percentage(50),
        Constraint::Percentage(50),
    ]).split(rows[1]);

    render_key_section(frame, " ⚡ Actions ", &help.actions, cols[0], &t, Color::Rgb(0, 120, 200));
    render_key_section(frame, " 🧭 Navigation ", &help.navigation, cols[1], &t, Color::Rgb(0, 160, 100));

    // ── Divider ─────────────────────────────────────────────────────────────
    frame.render_widget(
        Paragraph::new("─".repeat(inner.width as usize))
            .style(Style::default().fg(Color::Rgb(40, 40, 60))),
        rows[2],
    );

    // ── Tips ────────────────────────────────────────────────────────────────
    let tip_items: Vec<ListItem> = help.tips.iter()
        .map(|tip| {
            ListItem::new(Line::from(vec![
                Span::styled("  💡 ", Style::default().fg(Color::Yellow)),
                Span::styled(*tip, Style::default().fg(Color::Rgb(200, 200, 180))),
            ]))
        })
        .collect();

    let tips_list = List::new(tip_items)
        .block(
            Block::default()
                .title(" 💡 Tips ")
                .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(60, 60, 80)))
                .style(Style::default().bg(Color::Rgb(10, 10, 20))),
        );
    frame.render_widget(tips_list, rows[3]);

    // ── Close hint ─────────────────────────────────────────────────────────
    let hint = Paragraph::new(" [?] or [Esc] to close ")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Right);
    let hint_area = Rect {
        x: popup_area.x,
        y: popup_area.bottom().saturating_sub(1),
        width: popup_area.width.saturating_sub(2),
        height: 1,
    };
    frame.render_widget(hint, hint_area);
}

fn render_key_section(
    frame: &mut Frame,
    title: &str,
    entries: &[HelpEntry],
    area: Rect,
    _t: &Theme,
    accent: Color,
) {
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(accent).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(50, 50, 70)))
        .style(Style::default().bg(Color::Rgb(10, 10, 20)));

    let items: Vec<ListItem> = entries.iter().map(|e| {
        ListItem::new(Line::from(vec![
            Span::styled(
                format!("  {:12}", e.key),
                Style::default().fg(Color::Rgb(255, 200, 50)).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default()),
            Span::styled(e.desc, Style::default().fg(Color::Rgb(210, 210, 220))),
        ]))
    }).collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}
