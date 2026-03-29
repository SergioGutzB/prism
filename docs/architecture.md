# Architecture Overview

Prism is built in Rust using an asynchronous, event-driven architecture designed for high-concurrency LLM calls and a responsive Terminal User Interface (TUI).

## Core Components

### 1. The Orchestrator (`src/agents/orchestrator.rs`)
The "brain" of Prism. It coordinates the execution of multiple agents:
- **Phase-0 (Objective Validator):** Analyzes the PR description vs. the linked ticket and code diff to establish the *stated objectives*.
- **Phase-1 (Specialists):** Independent agents (Security, Architecture, Performance, etc.) run in parallel based on the `concurrency` setting.
- **Phase-2 (Summarizer):** Aggregates findings from Phase-1 and generates a cohesive review summary.

### 2. The Runner (`src/agents/runner.rs`)
Handles the low-level communication with LLM providers. It includes specific logic for:
- **Claude Code CLI Integration:** Uses `claude --print` to perform tasks without requiring an API key.
- **Gemini API Adapter:** Implements specialized **Combined Prompting** and **Automatic Retry (429)** logic for stable performance on Google AI Studio.
- **OpenAI/Anthropic APIs:** Standard JSON adapters for direct API access.

### 3. TUI & App State (`src/app.rs`, `src/ui/`)
Built using `ratatui`, the UI follows a "single source of truth" pattern in the `App` struct:
- **Event-Driven:** Key presses and background agent updates are sent through a `tokio::sync::mpsc` channel to the main loop.
- **Double-Check Screen:** An interactive stage where users can approve, reject, or edit AI-generated comments before publishing to GitHub.

### 4. Review Draft & De-duplication (`src/review/models.rs`)
The `ReviewDraft` struct manages the collection of approved findings. It includes a **Rust-native de-duplication engine** that groups similar findings on the same line using word-overlap similarity to prevent noise.

## Execution Workflow
1.  **PR Selection:** User selects a PR from the `PrList`.
2.  **Context Loading:** Prism fetches the full diff, PR metadata, and linked ticket information.
3.  **Agent Runners:** Orchestrator launches agents. Phase-0 runs first, then Phase-1 agents run according to the `concurrency` limit.
4.  **De-duplication:** As agents finish, findings are merged and de-duplicated in real-time.
5.  **Review Stage:** User manually approves/rejects comments in the `DoubleCheck` screen.
6.  **Publication:** Final review is pushed to GitHub via a single API call (Review + Inline Comments).
