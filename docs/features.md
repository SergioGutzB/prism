# Key Features

Prism is a tool built by engineers for engineers. It's packed with features designed to make code reviews more efficient and less noisy.

## 1. Multi-Agent Orchestration
Instead of a single AI trying to "review the whole PR," Prism breaks the task into specialized domains:
- **🎯 Objective Validator:** Checks if the code actually implements what the ticket/description says.
- **🔒 Security Reviewer:** Hunts for hardcoded secrets, injection points, and unsafe patterns.
- **⚡ Performance Reviewer:** Analyzes Big-O complexity, memory usage, and inefficient queries.
- **📝 Style & Readability:** Ensures adherence to naming conventions and project structure.

## 2. Interactive Double-Check Stage
Before any AI comments reach GitHub, you get to review them in Prism:
- **Approve/Reject:** Quickly filter out false positives.
- **Manual Editing:** Click on an AI comment to refine its wording or suggestion.
- **Hybrid Review:** Add your own manual comments directly in Prism to mix AI and human insights.

## 3. Rust-Native Semantic De-duplication
A sophisticated engine in `src/review/models.rs` ensures that multiple agents don't report the same issue twice.
- **Grouping:** Automatically groups comments on the same line.
- **Similarity Scoring:** Uses word-overlap analysis to identify near-identical suggestions.
- **Intelligent Pruning:** Keeps the most severe finding (e.g., Warning vs. Suggestion) or the human-written one.

## 4. Live Terminal Dashboard
A high-performance TUI (Terminal User Interface) built with `ratatui`:
- **Real-time Progress:** Watch as each agent completes its analysis.
- **Config Reload:** Hit `L` in the settings screen to instantly reload your `default.toml` or `.env` without restarting.
- **GitHub Sync:** Automatically fetches PR lists and handles the full review publication workflow.

## 5. Token Stats Tracker
Track your LLM usage in real-time. Prism estimates tokens for every request, helping you understand the cost of each review and identifying potential optimizations in your agent prompts.
