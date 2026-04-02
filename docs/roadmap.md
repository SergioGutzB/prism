# Roadmap & TODOs

Prism is an evolving project. Here are the planned features and areas for improvement.

## 🚀 Future Features (Planned)
- [x] **Custom Agent Creation Wizard:** A UI-driven way to add new reviewer agents without editing TOML files.
- [x] **Semantic Diff Analysis:** Instead of simple text diffs, use an AST-aware diff to provide more accurate context to the AI.
- [ ] **PR Thread Integration:** Ability to see and respond to existing GitHub comment threads directly from Prism.
- [ ] **Multi-PR Batching:** Select multiple PRs to review at once in the background.
- [ ] **Auto-Correct (The Fixer Agent):** Let the AI automatically generate a commit with the suggested fixes.

## 🛠️ Technical Improvements (Backlog)
- [ ] **Enhanced De-duplication:** Move from simple word-overlap to using local embeddings for true semantic similarity without LLM costs.
- [ ] **Plugin System:** Allow external Rust binaries to act as "Reviewer Agents" (e.g., wrap existing linters like `clippy` or `eslint`).
- [ ] **State Persistence:** Save your current draft (approved/rejected comments) so you can resume a review after closing the app.
- [ ] **Config Encryption:** Securely store API keys instead of relying purely on environment variables or plain TOML.

## 💡 Ideas?
If you have ideas for new agents or features, feel free to contribute! Prism is designed to be extensible.
