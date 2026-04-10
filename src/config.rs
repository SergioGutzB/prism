use config::{Config, Environment, File, FileFormat};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::collections::HashMap;

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
    #[serde(default)]
    pub editor: EditorConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EditorConfig {
    pub command: String,           // e.g. "nvim", "vim", "code --wait"
    pub internal_vim: bool,        // Use internal tui-textarea vim mode
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            command: "nvim".to_string(),
            internal_vim: true,
        }
    }
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

impl GitHubConfig {
    pub fn is_configured(&self) -> bool {
        !self.token.is_empty() && !self.owner.is_empty() && !self.repo.is_empty()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TicketsConfig {
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
    pub key_patterns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmConfig {
    pub provider: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
    /// Ollama context window size (num_ctx). 0 = use Ollama server default.
    /// Increase this if you see "truncating input prompt" warnings in the Ollama log.
    /// The model's max is shown as n_ctx_train in the log (e.g. 32768 for qwen2.5-coder).
    #[serde(default)]
    pub ollama_num_ctx: u32,
}

impl LlmConfig {
    /// Resolve the API key: config field takes priority, then environment variables.
    pub fn effective_api_key(&self) -> String {
        if !self.api_key.is_empty() {
            return self.api_key.clone();
        }
        match self.provider.as_str() {
            "anthropic" => std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            "openai" | "codex" => std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            "gemini" => std::env::var("GEMINI_API_KEY")
                .or_else(|_| std::env::var("GOOGLE_API_KEY"))
                .unwrap_or_default(),
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
    #[serde(default = "default_rigor")]
    pub review_rigor: String,
}

fn default_rigor() -> String {
    "moderate".to_string()
}

impl AgentsConfig {
    /// Returns a structured reasoning instruction to inject into the agent's user
    /// prompt (immediately before the Prompt Suffix). The instruction gives the
    /// model a concrete, per-comment decision rule rather than a vague directive,
    /// which leads to dramatically better filtering in practice.
    ///
    /// Returns `None` for `"moderate"` (the default — no extra constraint needed).
    pub fn rigor_instruction(&self) -> Option<&'static str> {
        match self.review_rigor.as_str() {
            "critical_only" => Some(
                "## Review Rigor: CRITICAL ONLY\n\
                 \n\
                 You are operating in **critical-only** mode. Before adding any comment \
                 to your output, apply this decision test:\n\
                 \n\
                 **Include a comment ONLY if it meets at least one of these criteria:**\n\
                 1. It is a security vulnerability (injection, data exposure, authentication bypass, \
                    privilege escalation, hardcoded secret)\n\
                 2. It causes incorrect program behavior (wrong output, crash, data corruption, \
                    race condition, integer overflow)\n\
                 3. It prevents the code from building or running\n\
                 4. It causes data loss or irreversible state corruption\n\
                 \n\
                 **Skip everything else without exception**, including:\n\
                 - Style, naming, or readability observations\n\
                 - Documentation gaps\n\
                 - Performance optimizations\n\
                 - Test coverage suggestions\n\
                 - Architectural preferences\n\
                 - \"Could be cleaner\" observations\n\
                 \n\
                 **Severity constraint:** Only emit `\"critical\"` severity. \
                 If an issue qualifies but feels like a `\"warning\"`, still emit it as \
                 `\"critical\"` — that is the only valid severity in this mode.\n\
                 \n\
                 > Decision question: *\"Would a user or the running system be materially harmed \
                 by this code as written?\"* If the honest answer is NO → skip the comment.",
            ),
            "strict" => Some(
                "## Review Rigor: STRICT\n\
                 \n\
                 You are operating in **strict** mode. For each potential finding, apply \
                 this decision test:\n\
                 \n\
                 **Include a comment ONLY if it meets at least one of these criteria:**\n\
                 1. Security vulnerability of any kind\n\
                 2. Incorrect behavior, crash, or data corruption\n\
                 3. Missing error handling for realistic (not hypothetical) failure cases\n\
                 4. Significant architectural flaw that will cause maintenance or reliability \
                    problems in practice\n\
                 5. Missing test coverage for critical or error-handling code paths\n\
                 6. Serious performance issue (O(n²) in a hot path, N+1 query pattern, \
                    memory/resource leak)\n\
                 7. Code that visibly contradicts the project's established conventions \
                    (patterns you can see in the diff context, not hypothetical ones)\n\
                 \n\
                 **Skip everything else**, including:\n\
                 - Cosmetic style preferences when current code is already clear\n\
                 - Naming improvements when current naming is not misleading\n\
                 - Nice-to-have documentation on non-public APIs\n\
                 - Micro-optimizations in non-critical paths\n\
                 - Hypothetical future concerns\n\
                 \n\
                 **Severity constraint:** Only emit `\"critical\"` or `\"warning\"` severity. \
                 Do not emit `\"suggestion\"` or `\"praise\"`.\n\
                 \n\
                 > Decision question: *\"Does this issue meaningfully degrade reliability, \
                 security, or correctness in a way that will matter in production?\"* \
                 If NO → skip the comment.",
            ),
            "light" => Some(
                "## Review Rigor: LIGHT\n\
                 \n\
                 You are operating in **light** mode. Be encouraging and constructive:\n\
                 \n\
                 - Flag all genuine issues (security, correctness, serious performance)\n\
                 - For style and naming: only comment if it would genuinely confuse a \
                   new reader — skip preferences\n\
                 - Actively use `\"praise\"` for good patterns, clean abstractions, \
                   and well-written tests — good code deserves acknowledgment\n\
                 - Phrase suggestions gently (prefer *\"consider...\"* over *\"should\"* \
                   or *\"must\"*)\n\
                 - Aim for high signal-to-noise: if you are on the fence about a minor \
                   observation, skip it\n\
                 \n\
                 > Goal: the author should finish reading your review feeling informed \
                 and motivated, not overwhelmed.",
            ),
            _ => None, // "moderate" — standard review, no extra constraint
        }
    }

    /// Returns the minimum `Severity` a comment must have to be kept after
    /// generation under the current rigor level. Used as a post-generation
    /// safety net in case the model ignores the rigor instruction.
    ///
    /// Returns `None` for levels where no filtering is needed (`moderate`, `light`).
    pub fn min_severity_for_rigor(&self) -> Option<crate::review::models::Severity> {
        use crate::review::models::Severity;
        match self.review_rigor.as_str() {
            "critical_only" => Some(Severity::Critical),
            "strict"        => Some(Severity::Warning),
            _               => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UiConfig {
    pub theme: String,
    pub diff_context_lines: usize,
    pub show_line_numbers: bool,
    pub highlight_syntax: bool,
    pub keybindings: KeybindingsConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KeybindingsConfig {
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
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let config_path = PathBuf::from(home)
            .join(".config")
            .join("prism")
            .join("config.toml");

        let mut s = Config::builder();

        // 1. Start with hardcoded defaults (optional, could be in a separate file)
        s = s.add_source(File::from_str(
            include_str!("../config/default.toml"),
            FileFormat::Toml,
        ));

        // 2. Load from ~/.config/prism/config.toml
        if config_path.exists() {
            s = s.add_source(File::from(config_path));
        }

        // 3. Environment variables (PRISM_GITHUB__TOKEN etc.)
        s = s.add_source(Environment::with_prefix("PRISM").separator("__"));

        let cfg: AppConfig = s.build()?.try_deserialize()?;
        Ok(cfg)
    }

    pub fn gh_token() -> Option<String> {
        std::process::Command::new("gh")
            .args(["auth", "token"])
            .output()
            .ok()
            .and_then(|output| {
                if output.status.success() {
                    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
                } else {
                    None
                }
            })
    }

    pub fn gh_current_repo() -> Option<(String, String)> {
        std::process::Command::new("gh")
            .args(["repo", "view", "--json", "owner,name", "--template", "{{.owner.login}}:{{.name}}"])
            .output()
            .ok()
            .and_then(|output| {
                if output.status.success() {
                    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let parts: Vec<&str> = s.split(':').collect();
                    if parts.len() == 2 {
                        Some((parts[0].to_string(), parts[1].to_string()))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
    }

    pub fn is_github_configured(&self) -> bool {
        !self.github.token.is_empty() && !self.github.owner.is_empty() && !self.github.repo.is_empty()
    }

    pub fn is_llm_configured(&self) -> bool {
        self.llm.effective_api_key().len() > 5 || self.llm.provider == "claude-cli" || self.llm.provider == "ollama"
    }

    pub fn save_github_config(token: &str, owner: &str, repo: &str) -> Result<()> {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let config_dir = PathBuf::from(home).join(".config").join("prism");
        let config_path = config_dir.join("config.toml");

        if !config_dir.exists() {
            std::fs::create_dir_all(&config_dir)?;
        }

        let mut doc = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            content.parse::<toml_edit::DocumentMut>().unwrap_or_default()
        } else {
            toml_edit::DocumentMut::new()
        };

        let github = &mut doc["github"];
        github["token"] = toml_edit::value(token);
        github["owner"] = toml_edit::value(owner);
        github["repo"] = toml_edit::value(repo);

        std::fs::write(config_path, doc.to_string())?;
        Ok(())
    }

    pub fn save_user_config(&self) -> Result<()> {
        let path = dirs_user_config();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let doc = toml::to_string_pretty(&self)?;
        std::fs::write(path, doc)?;
        Ok(())
    }

    pub fn load_stats() -> HashMap<String, crate::app::ModelStats> {
        let path = dirs_stats_file();
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(path) {
                return serde_json::from_str(&content).unwrap_or_default();
            }
        }
        HashMap::new()
    }

    pub fn save_stats(stats: &HashMap<String, crate::app::ModelStats>) {
        let path = dirs_stats_file();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(content) = serde_json::to_string_pretty(stats) {
            let _ = std::fs::write(path, content);
        }
    }
}

fn dirs_user_config() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("prism").join("config.toml")
}

fn dirs_stats_file() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("prism").join("stats.json")
}
