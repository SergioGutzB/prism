# Prism - Code Review Documentation

Prism is a high-performance, multi-agent AI code review tool for GitHub. It orchestrates multiple specialized AI agents to analyze pull requests, identify security risks, architectural flaws, and style inconsistencies, and then groups them into a single, cohesive review.

## Documentation Index

- [Architecture Overview](architecture.md): System design, agent orchestration, and the TUI event loop.
- [AI & LLM Configuration](llm_configuration.md): Detailed guide for Gemini, Claude, and OpenAI integration, including retry logic and compatibility strategies.
- [Key Features](features.md): In-depth look at multi-agent reviews, semantic de-duplication, and TUI capabilities.
- [Performance & Tokens](performance_and_tokens.md): Concurrency, token consumption optimization, and Rust-based filtering.
- [Configuration Guide](configuration.md): Reference for `default.toml` and environment variables.
- [Roadmap & TODOs](roadmap.md): Future improvements and planned features.

## Project Vision
Prism aims to provide "Human-in-the-loop" AI code reviews that are as thorough as a senior engineer but as fast as a CI job. By using a specialized set of agents instead of a single general-purpose model, Prism achieves higher precision and lower noise.
