use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::{debug, warn};

use crate::agents::models::{AgentContext, AgentDefinition, AgentLlmOverride, AgentMeta, AgentPrompt};
use crate::config::AppConfig;

// ── Built-in agents (embedded at compile time) ────────────────────────────────

const BUILTIN_OBJECTIVE:    &str = include_str!("../../agents/objective.md");
const BUILTIN_SECURITY:     &str = include_str!("../../agents/security.md");
const BUILTIN_ARCHITECTURE: &str = include_str!("../../agents/architecture.md");
const BUILTIN_TESTS:        &str = include_str!("../../agents/tests.md");
const BUILTIN_PERFORMANCE:  &str = include_str!("../../agents/performance.md");
const BUILTIN_STYLE:        &str = include_str!("../../agents/style.md");
const BUILTIN_SUMMARY:      &str = include_str!("../../agents/summary.md");

const BUILTINS: &[&str] = &[
    BUILTIN_OBJECTIVE,
    BUILTIN_SECURITY,
    BUILTIN_ARCHITECTURE,
    BUILTIN_TESTS,
    BUILTIN_PERFORMANCE,
    BUILTIN_STYLE,
    BUILTIN_SUMMARY,
];

// ── Public API ────────────────────────────────────────────────────────────────

/// Load all agent definitions.
///
/// Built-in agents (`.md` files in `agents/`) are always loaded first.
/// User-defined agents from `~/.config/prism/agents/` override built-ins that
/// share the same `id`. Both `.md` and `.toml` formats are accepted in the
/// user directory for backward compatibility.
///
/// The final list is sorted by `order`.
pub fn load_agents(config: &AppConfig) -> Result<Vec<AgentDefinition>> {
    let mut agents: Vec<AgentDefinition> = Vec::new();

    // Load built-ins (Markdown format)
    for source in BUILTINS {
        match parse_agent(source, "built-in") {
            Ok(def) => {
                debug!("Loaded built-in agent: {}", def.agent.id);
                agents.push(def);
            }
            Err(e) => {
                warn!("Failed to parse built-in agent: {e}");
            }
        }
    }

    // Load user overrides from the configured directory
    let user_agents_dir = expand_tilde(&config.agents.agents_dir);
    if user_agents_dir.exists() {
        match std::fs::read_dir(&user_agents_dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let ext = path.extension().and_then(|e| e.to_str());
                    if !matches!(ext, Some("md") | Some("toml")) {
                        continue;
                    }

                    match std::fs::read_to_string(&path) {
                        Ok(content) => match parse_agent(&content, &path.display().to_string()) {
                            Ok(def) => {
                                debug!("Loaded user agent: {} from {}", def.agent.id, path.display());
                                // Replace built-in with same id if present
                                if let Some(existing) = agents.iter_mut().find(|a| a.agent.id == def.agent.id) {
                                    *existing = def;
                                } else {
                                    agents.push(def);
                                }
                            }
                            Err(e) => warn!("Failed to parse {}: {e}", path.display()),
                        },
                        Err(e) => warn!("Failed to read {}: {e}", path.display()),
                    }
                }
            }
            Err(e) => warn!("Cannot read agents directory {}: {e}", user_agents_dir.display()),
        }
    }

    agents.sort_by_key(|a| a.agent.order);
    Ok(agents)
}

// ── Parsing ───────────────────────────────────────────────────────────────────

/// Parse an agent definition from either Markdown (`.md`) or TOML (`.toml`) source.
fn parse_agent(source: &str, label: &str) -> Result<AgentDefinition> {
    let trimmed = source.trim_start();
    if trimmed.starts_with("---") {
        parse_agent_md(source).with_context(|| format!("Markdown parse error in {label}"))
    } else {
        toml::from_str(source).with_context(|| format!("TOML parse error in {label}"))
    }
}

// ── Markdown parser ───────────────────────────────────────────────────────────

/// Intermediate struct for deserializing the YAML frontmatter of a `.md` agent file.
///
/// This mirrors `AgentMeta` but without the `prompt` field — the system prompt
/// and prompt suffix live in the Markdown body as `## System Prompt` and
/// `## Prompt Suffix` sections.
#[derive(Debug, Deserialize)]
struct AgentFrontmatter {
    id: String,
    name: String,
    description: String,
    enabled: bool,
    order: u32,
    icon: String,
    color: String,
    #[serde(default)]
    phase_zero: bool,
    #[serde(default)]
    synthesis: bool,
    #[serde(default)]
    llm: Option<AgentLlmOverride>,
    context: AgentContext,
    /// Per-agent minimum severity override. When set, discards comments below
    /// this threshold regardless of the global `review_rigor` setting.
    /// Accepts: "praise" | "suggestion" | "warning" | "critical"
    #[serde(default)]
    min_severity: Option<String>,
}

/// Parse a Markdown agent file into an `AgentDefinition`.
///
/// Expected format:
/// ```text
/// ---
/// <YAML frontmatter — metadata, context, optional llm overrides>
/// ---
///
/// ## System Prompt
///
/// <system prompt text — plain Markdown, no escaping needed>
///
/// ## Prompt Suffix
///
/// <prompt suffix text>
/// ```
fn parse_agent_md(source: &str) -> Result<AgentDefinition> {
    // ── Extract frontmatter ────────────────────────────────────────────────
    let after_open = source
        .trim_start()
        .strip_prefix("---")
        .and_then(|s| s.strip_prefix('\n').or_else(|| s.strip_prefix("\r\n")))
        .context("Agent .md must start with a '---' frontmatter block")?;

    let close_idx = after_open
        .find("\n---")
        .context("Agent .md missing closing '---' for frontmatter")?;

    let frontmatter_str = &after_open[..close_idx];
    // Everything after the closing "---" (plus newline)
    let body = after_open[close_idx..]
        .trim_start_matches('\n')
        .trim_start_matches("---")
        .trim_start_matches('\n');

    // ── Deserialize frontmatter ────────────────────────────────────────────
    let fm: AgentFrontmatter = serde_yaml::from_str(frontmatter_str)
        .context("Failed to parse YAML frontmatter")?;

    // ── Extract body sections ──────────────────────────────────────────────
    let system = extract_section(body, "System Prompt")
        .context("Agent .md missing '## System Prompt' section")?;

    let prompt_suffix = extract_section(body, "Prompt Suffix")
        .unwrap_or_default();

    Ok(AgentDefinition {
        agent: AgentMeta {
            id: fm.id,
            name: fm.name,
            description: fm.description,
            enabled: fm.enabled,
            order: fm.order,
            icon: fm.icon,
            color: fm.color,
            phase_zero: fm.phase_zero,
            synthesis: fm.synthesis,
            llm: fm.llm,
            context: fm.context,
            min_severity: fm.min_severity,
            prompt: AgentPrompt { system, prompt_suffix },
        },
    })
}

/// Extract the text content of a `## <heading>` section from a Markdown body.
///
/// The section ends at the next `## ` heading or at the end of the string.
/// Leading/trailing whitespace is trimmed.
fn extract_section(body: &str, heading: &str) -> Option<String> {
    let marker = format!("## {heading}");
    let start = body.find(&marker)?;
    let after_heading = &body[start + marker.len()..];
    // Skip the newline immediately after the heading line
    let content_start = after_heading
        .find('\n')
        .map(|i| i + 1)
        .unwrap_or(after_heading.len());
    let content = &after_heading[content_start..];
    // End at the next "## " section heading
    let end = content.find("\n## ").unwrap_or(content.len());
    let text = content[..end].trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(stripped)
    } else {
        PathBuf::from(path)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_MD: &str = r#"---
id: test-agent
name: Test Agent
description: A sample agent for unit tests.
enabled: true
order: 99
icon: "🧪"
color: green
synthesis: false

context:
  include_diff: true
  include_pr_description: false
  include_ticket: false
  include_file_list: false
  exclude_patterns:
    - "*.md"
  include_patterns: []
---

## System Prompt

You are a test agent. Do nothing useful.

Respond with `[]`.

## Prompt Suffix

Return an empty JSON array.
"#;

    #[test]
    fn parses_markdown_agent() {
        let def = parse_agent_md(SAMPLE_MD).expect("Should parse");
        assert_eq!(def.agent.id, "test-agent");
        assert_eq!(def.agent.name, "Test Agent");
        assert_eq!(def.agent.order, 99);
        assert!(!def.agent.synthesis);
        assert!(def.agent.context.include_diff);
        assert!(!def.agent.context.include_pr_description);
        assert_eq!(def.agent.context.exclude_patterns, vec!["*.md"]);
        assert!(def.agent.prompt.system.contains("You are a test agent"));
        assert!(def.agent.prompt.prompt_suffix.contains("empty JSON array"));
    }

    #[test]
    fn parses_builtin_agents() {
        for (name, src) in [
            ("objective", BUILTIN_OBJECTIVE),
            ("security", BUILTIN_SECURITY),
            ("architecture", BUILTIN_ARCHITECTURE),
            ("tests", BUILTIN_TESTS),
            ("performance", BUILTIN_PERFORMANCE),
            ("style", BUILTIN_STYLE),
            ("summary", BUILTIN_SUMMARY),
        ] {
            let def = parse_agent(src, name)
                .unwrap_or_else(|e| panic!("Failed to parse built-in agent '{name}': {e}"));
            assert_eq!(def.agent.id, name, "Agent id mismatch for {name}");
            assert!(!def.agent.prompt.system.is_empty(), "Empty system prompt for {name}");
        }
    }

    #[test]
    fn summary_is_synthesis() {
        let def = parse_agent(BUILTIN_SUMMARY, "summary").unwrap();
        assert!(def.agent.synthesis, "summary agent must have synthesis = true");
    }

    #[test]
    fn objective_is_phase_zero() {
        let def = parse_agent(BUILTIN_OBJECTIVE, "objective").unwrap();
        assert!(def.agent.phase_zero, "objective agent must have phase_zero = true");
        assert!(!def.agent.synthesis, "objective agent must not be a synthesis agent");
        assert_eq!(def.agent.order, 0, "objective agent must have order = 0");
    }
}
