use config::{Config, Environment, File, FileFormat};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::Result;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    pub github: GitHubConfig,
    pub tickets: TicketsConfig,
    pub llm: LlmConfig,
    pub agents: AgentsConfig,
    pub ui: UiConfig,
    #[serde(default)]
    pub publishing: PublishingConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PublishingConfig {
    pub confirm_before_publish: bool,
    pub auto_translate_to_english: bool,
    pub auto_correct_grammar: bool,
    pub format: ReviewFormatConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReviewFormatConfig {
    /// Template for the overall review body.
    /// Variables: {pr_number}, {pr_title}, {comment_count},
    /// {critical_count}, {warning_count}, {suggestion_count}, {praise_count}, {comments_list}
    pub body_template: String,
    /// Template for each comment entry in the list.
    /// Variables: {file}, {line}, {severity}, {body}, {source}
    pub comment_template: String,
}

impl Default for PublishingConfig {
    fn default() -> Self {
        Self {
            confirm_before_publish: true,
            auto_translate_to_english: false,
            auto_correct_grammar: false,
            format: ReviewFormatConfig::default(),
        }
    }
}

impl Default for ReviewFormatConfig {
    fn default() -> Self {
        Self {
            body_template: "## Code Review\n\n{comment_count} comment(s) for PR #{pr_number}.\n\n{comments_list}".to_string(),
            comment_template: "- **`{file}:{line}`** [{severity}] {body}".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitHubConfig {
    pub token: String,
    pub owner: String,
    pub repo: String,
    pub per_page: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TicketsConfig {
    #[serde(default)]
    pub providers: Vec<TicketProviderConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TicketProviderConfig {
    #[serde(rename = "type")]
    pub provider_type: String,
    pub enabled: bool,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub api_token: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub key_patterns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmConfig {
    /// LLM backend. Supported values:
    /// "claude-cli"  — Claude Code CLI (`claude --print`), no API key needed
    /// "anthropic"   — Direct Anthropic Messages API (ANTHROPIC_API_KEY)
    /// "openai"      — OpenAI Chat Completions API (OPENAI_API_KEY)
    /// "gemini"      — Google Gemini API (GEMINI_API_KEY or GOOGLE_API_KEY)
    /// "ollama"      — Local Ollama server (no key, set base_url if non-default)
    pub provider: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    /// API key. If empty, falls back to the provider's standard env var.
    #[serde(default)]
    pub api_key: String,
    /// Override the API base URL (e.g. Azure OpenAI endpoint, custom Ollama host).
    /// Leave empty to use the provider's default endpoint.
    #[serde(default)]
    pub base_url: String,
}

impl LlmConfig {
    /// Resolve the API key: config field takes priority, then env var.
    pub fn effective_api_key(&self) -> String {
        if !self.api_key.is_empty() {
            return self.api_key.clone();
        }
        match self.provider.as_str() {
            "anthropic" | "claude" | "claude-cli" => {
                std::env::var("ANTHROPIC_API_KEY").unwrap_or_default()
            }
            "openai" | "codex" => {
                std::env::var("OPENAI_API_KEY").unwrap_or_default()
            }
            "gemini" => {
                std::env::var("GEMINI_API_KEY")
                    .or_else(|_| std::env::var("GOOGLE_API_KEY"))
                    .unwrap_or_default()
            }
            _ => String::new(),
        }
    }

    /// Resolve the base URL: config field takes priority, then provider default.
    pub fn effective_base_url(&self) -> &str {
        if !self.base_url.is_empty() {
            return &self.base_url;
        }
        match self.provider.as_str() {
            "anthropic" => "https://api.anthropic.com",
            "openai" | "codex" => "https://api.openai.com",
            "gemini" => "https://generativelanguage.googleapis.com",
            "ollama" => "http://localhost:11434",
            _ => "",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentsConfig {
    pub agents_dir: String,
    pub concurrency: usize,
    pub timeout_secs: u64,
    #[serde(default)]
    pub diff_exclude_patterns: Vec<String>,
    pub max_diff_tokens: u32,
    /// Review strictness level. Controls how much the AI focuses on issues.
    /// Values: "critical_only" | "strict" | "moderate" | "light"
    #[serde(default = "default_review_rigor")]
    pub review_rigor: String,
}

fn default_review_rigor() -> String {
    "moderate".to_string()
}

impl AgentsConfig {
    /// Returns a system-prompt prefix that adjusts agent behavior based on rigor level.
    pub fn rigor_prefix(&self) -> &'static str {
        match self.review_rigor.as_str() {
            "critical_only" => {
                "IMPORTANT CONSTRAINT: Only report CRITICAL issues — security vulnerabilities, \
                 data-loss bugs, crashes, or breaking API changes. If nothing critical is found, \
                 return an empty array []. Do NOT report style issues, minor suggestions, or \
                 anything that does not have a direct functional impact.\n\n"
            }
            "strict" => {
                "REVIEW MODE: Strict. Be thorough and rigorous. Flag all potential issues \
                 including style inconsistencies, performance concerns, maintainability problems, \
                 and correctness issues. Include minor suggestions where relevant.\n\n"
            }
            "light" => {
                "REVIEW MODE: Light. Focus only on significant bugs and clear improvements. \
                 Avoid nitpicking style, naming, or trivial formatting. Only report issues \
                 that clearly matter for correctness or maintainability.\n\n"
            }
            _ => {
                // "moderate" — no extra prefix, default agent behavior
                ""
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UiConfig {
    pub theme: String,
    pub diff_context_lines: u32,
    pub show_line_numbers: bool,
    pub highlight_syntax: bool,
    pub keybindings: KeyBindings,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KeyBindings {
    pub quit: String,
    pub back: String,
    pub confirm: String,
    pub next_pane: String,
    pub prev_pane: String,
    pub generate_review: String,
    pub manual_comment: String,
    pub publish: String,
    pub toggle_item: String,
    pub select_all: String,
    pub deselect_all: String,
    pub open_browser: String,
    pub refresh: String,
    pub search: String,
    pub agent_config: String,
    pub settings: String,
    pub preview_summary: String,
    pub check_file: String,
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let default_config = include_str!("../config/default.toml");

        let user_config_path = dirs_user_config();

        let mut builder = Config::builder()
            .add_source(File::from_str(default_config, FileFormat::Toml))
            .add_source(
                File::with_name(
                    user_config_path
                        .to_str()
                        .unwrap_or("~/.config/prism/config"),
                )
                .required(false),
            )
            // GitHub env vars
            .add_source(
                Environment::with_prefix("GITHUB")
                    .prefix_separator("_")
                    .separator("__")
                    .try_parsing(true),
            )
            // Jira env vars
            .add_source(
                Environment::with_prefix("JIRA")
                    .prefix_separator("_")
                    .separator("__")
                    .try_parsing(true),
            )
            // Linear env vars
            .add_source(
                Environment::with_prefix("LINEAR")
                    .prefix_separator("_")
                    .separator("__")
                    .try_parsing(true),
            )
            // Anthropic / OpenAI
            .add_source(
                Environment::with_prefix("ANTHROPIC")
                    .prefix_separator("_")
                    .separator("__")
                    .try_parsing(true),
            )
            .add_source(
                Environment::with_prefix("OPENAI")
                    .prefix_separator("_")
                    .separator("__")
                    .try_parsing(true),
            );

        // Override specific keys from individual env vars
        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            builder = builder.set_override("github.token", token)?;
        }
        if let Ok(owner) = std::env::var("GITHUB_OWNER") {
            builder = builder.set_override("github.owner", owner)?;
        }
        if let Ok(repo) = std::env::var("GITHUB_REPO") {
            builder = builder.set_override("github.repo", repo)?;
        }
        if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
            builder = builder.set_override("llm.api_key", api_key)?;
        }

        let cfg = builder.build()?;
        let app_config: AppConfig = cfg.try_deserialize()?;
        Ok(app_config)
    }

    pub fn is_github_configured(&self) -> bool {
        !self.github.token.is_empty()
            && !self.github.owner.is_empty()
            && !self.github.repo.is_empty()
    }

    pub fn is_llm_configured(&self) -> bool {
        match self.llm.provider.as_str() {
            "claude-cli" | "claude" => Self::is_claude_cli_available(),
            "ollama" => true, // local, no key required
            _ => !self.llm.effective_api_key().is_empty(),
        }
    }

    /// Check if the `claude` CLI is available in PATH.
    pub fn is_claude_cli_available() -> bool {
        std::process::Command::new("claude")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Try to get a GitHub token from the `gh` CLI (`gh auth token`).
    pub fn gh_token() -> Option<String> {
        let out = std::process::Command::new("gh")
            .args(["auth", "token"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let token = String::from_utf8(out.stdout).ok()?.trim().to_string();
        if token.is_empty() { None } else { Some(token) }
    }

    /// Try to detect owner/repo from the current git remote via `gh repo view`.
    pub fn gh_current_repo() -> Option<(String, String)> {
        let out = std::process::Command::new("gh")
            .args(["repo", "view", "--json", "owner,name"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        let owner = v["owner"]["login"].as_str()?.to_string();
        let name  = v["name"].as_str()?.to_string();
        Some((owner, name))
    }

    /// Persist GitHub credentials to `~/.config/prism/config.toml`.
    /// Creates the file if it doesn't exist; merges with existing content.
    pub fn save_github_config(token: &str, owner: &str, repo: &str) -> anyhow::Result<()> {
        let path = dirs_user_config();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }

        // Read existing TOML (if any) so we don't lose other settings
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let mut doc: toml::Table = existing.parse().unwrap_or_default();

        let github = doc
            .entry("github")
            .or_insert(toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .cloned()
            .unwrap_or_default();

        let mut gh = github;
        gh.insert("token".into(), toml::Value::String(token.to_string()));
        gh.insert("owner".into(), toml::Value::String(owner.to_string()));
        gh.insert("repo".into(),  toml::Value::String(repo.to_string()));
        doc.insert("github".into(), toml::Value::Table(gh));

        std::fs::write(&path, toml::to_string_pretty(&doc)?)?;
        Ok(())
    }
}

fn dirs_user_config() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("prism").join("config.toml")
}
