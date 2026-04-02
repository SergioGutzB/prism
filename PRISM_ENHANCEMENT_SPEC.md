# PRISM Enhancement Specification for Claude Code

This document outlines the desired improvements and new features for the PRISM TUI. The goal is to implement these features with high performance (Rust-idiomatic) and zero regressions in the existing TUI state machine.

---

## 1. Professional Vim-style Editing
### Objectives
- Replace standard `String` inputs with a robust editor for `ReviewCompose` and `AgentWizard`.
- Support internal Vim-lite navigation (`hjkl`, `w`, `b`, `i`, `Esc`).
- Support spawning the external system editor (`nvim`, `vim`, `$EDITOR`).

### Technical Requirements
- **Library:** Use `tui-textarea`.
- **Internal State:** Wrap `TextArea` in a `PrismEditor` struct in `src/ui/editor.rs`.
- **Event Loop Fix:** In `src/main.rs`, the `KeySequenceDetector` (which handles sequences like `gg`) **must be bypassed** when the application is in `InputMode::Insert`. This prevents keys like `n` or `i` from being intercepted.
- **External Editor:**
    - Use `tempfile` to create a file in `/dev/shm` (Linux RAM-disk) or `/tmp`.
    - Suspend TUI mode (`disable_raw_mode`, `LeaveAlternateScreen`).
    - Spawn `$EDITOR` as a synchronous child process.
    - Re-enable TUI and read the file back.

---

## 2. Robust Gemini Pro Integration
### Objectives
- Eliminate `429 Too Many Requests` and `404/400 Bad Request` errors.

### Technical Requirements
- **Strategy:** Implement "Combined Prompting". Merge `system_instruction` directly into the `user` message: `format!("INSTRUCTIONS:\n{}\n\nCONTEXT:\n{}", system, prompt)`.
- **Endpoint:** Use the `v1beta` endpoint for AI Studio compatibility.
- **Retry Logic:** Add a loop in `src/agents/runner.rs` that catches status code `429`, sleeps for 2 seconds, and retries up to 3 times using `tokio::time::sleep`.

---

## 3. Rust-Native Semantic De-duplication
### Objectives
- Prevent duplicate comments on the same line from different agents (e.g., Security and Style both flagging a secret).

### Technical Requirements
- **Location:** `src/review/models.rs`, method `add_comment`.
- **Logic:** 
    1. If a new comment is on the same `file_path` and `line` as an existing one.
    2. Calculate word-overlap similarity (Jaccard).
    3. If similarity > 50%:
        - Keep **Manual** over **Agent**.
        - Keep higher **Severity** (Critical > Warning > Suggestion).
        - Keep the longer/more detailed description.

---

## 4. Custom Agent Creation Wizard
### Objectives
- A TUI screen to create new reviewers without manual file editing.

### Technical Requirements
- **Fields:** `ID` (slug), `Name`, `Icon` (emoji), `System Prompt`.
- **Persistence:** Save as a Markdown file with YAML frontmatter in the configured `agents_dir`.
- **Hot-Reload:** After saving, immediately trigger `agents::loader::load_agents` to update the `App` state.

---

## 5. Semantic Diff Analysis
### Objectives
- Provide agents with context about *what* function or class is being modified.

### Technical Requirements
- **Logic:** Parse the `@@` hunk headers in the unified diff. Extract the text after the line numbers (which usually contains the function signature).
- **Injection:** Prepend `[Context: function_name()]` to each hunk before sending the diff to the LLM context.

---

## 6. Historical Model Statistics
### Objectives
- Track usage across all models (not just Claude) and persist data.

### Technical Requirements
- **Storage:** A JSON file `~/.config/prism/stats.json`.
- **Metrics:** `calls`, `input_tokens`, `output_tokens`, and `start_date`.
- **UI:** Update `token_stats.rs` to show a breakdown table of all models used historically.

---

## 7. Critical UI State Machine Rules
- **Layered Esc:** `Esc` should:
    1. Close Popups if open.
    2. Then, close Help/Stats overlays if open.
    3. Then, exit `Insert` mode if active.
    4. Finally, navigate `Back` in the screen stack.
- **Async Safety:** Ensure all GitHub API calls are `await`ed and handled inside `tokio::spawn` to keep the TUI thread at 60 FPS.
