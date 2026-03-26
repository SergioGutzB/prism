use std::time::Instant;

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::{debug, warn};

use crate::agents::context::ReviewContext;
use crate::agents::models::{AgentDefinition, AgentStatus};
use crate::config::AppConfig;
use crate::review::models::{CommentSource, GeneratedComment, Severity};

/// Raw JSON shape that every agent (except summary) must return.
#[derive(Debug, Deserialize)]
struct RawComment {
    file_path: Option<String>,
    line: Option<u32>,
    body: String,
    severity: Option<String>,
}

/// Summary agent returns a single object.
#[derive(Debug, Deserialize)]
struct RawSummary {
    body: String,
    severity: Option<String>,
}

pub struct AgentRunner {
    config: AppConfig,
}

impl AgentRunner {
    pub fn new(config: AppConfig) -> Self {
        Self { config }
    }

    /// Execute a single agent against the given review context.
    /// Returns an `AgentStatus` — never panics.
    pub async fn run(
        &self,
        agent: &AgentDefinition,
        ctx: &ReviewContext,
    ) -> AgentStatus {
        if !agent.agent.enabled {
            return AgentStatus::Disabled;
        }

        let _started_at = chrono::Utc::now();
        let start = Instant::now();

        match self.run_inner(agent, ctx).await {
            Ok(comments) => {
                let elapsed_ms = start.elapsed().as_millis() as u64;
                AgentStatus::Done { comments, elapsed_ms }
            }
            Err(e) => AgentStatus::Failed {
                error: format!("{:#}", e),
            },
        }
    }

    async fn run_inner(
        &self,
        agent: &AgentDefinition,
        ctx: &ReviewContext,
    ) -> Result<Vec<GeneratedComment>> {
        // Build the prompt
        let prompt = self.build_prompt(agent, ctx);
        debug!(agent_id = %agent.agent.id, prompt_len = prompt.len(), "Running agent");

        // Attempt LLM call with 1 retry on JSON parse failure
        let raw_response = self.call_llm(agent, &prompt).await?;

        match self.parse_response(agent, &raw_response) {
            Ok(comments) => Ok(comments),
            Err(parse_err) => {
                warn!(
                    agent_id = %agent.agent.id,
                    "First parse attempt failed ({}), retrying",
                    parse_err
                );
                let retry_prompt = format!(
                    "{}\n\nYour previous response could not be parsed as JSON. \
                     Please respond ONLY with valid JSON, no markdown fences.",
                    prompt
                );
                let retry_response = self.call_llm(agent, &retry_prompt).await?;
                self.parse_response(agent, &retry_response)
                    .context("JSON parse failed after retry")
            }
        }
    }

    fn build_prompt(&self, agent: &AgentDefinition, ctx: &ReviewContext) -> String {
        let mut parts: Vec<String> = Vec::new();

        if agent.agent.context.include_pr_description {
            parts.push(format!(
                "## PR: {} (#{}) by {}\n\n{}",
                ctx.pr_title, ctx.pr_number, ctx.pr_author, ctx.pr_description
            ));
        }

        if agent.agent.context.include_ticket {
            if let Some(ticket) = &ctx.ticket {
                parts.push(format!(
                    "## Linked Ticket: {} — {}\n\n{}",
                    ticket.key, ticket.title, ticket.description.as_deref().unwrap_or("")
                ));
            }
        }

        if agent.agent.context.include_file_list {
            parts.push(format!("## Changed Files\n\n{}", ctx.file_list_text()));
        }

        if agent.agent.context.include_diff {
            let diff = ctx.truncated_diff(self.config.agents.max_diff_tokens);
            parts.push(format!("## Diff\n\n```diff\n{}\n```", diff));
        }

        parts.push(agent.agent.prompt.prompt_suffix.clone());
        parts.join("\n\n")
    }

    async fn call_llm(
        &self,
        agent: &AgentDefinition,
        prompt: &str,
    ) -> Result<String> {
        use std::process::Stdio;
        use tokio::io::AsyncWriteExt;
        use tokio::process::Command;

        let timeout_secs = self.config.agents.timeout_secs;
        let system_prompt = &agent.agent.prompt.system;

        let mut child = Command::new("claude")
            .arg("--print")
            .arg("--system-prompt")
            .arg(system_prompt)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn `claude` — is Claude Code installed?")?;

        // Write user prompt to stdin, then close it to signal EOF
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(prompt.as_bytes())
                .await
                .context("Failed to write prompt to claude stdin")?;
        }

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            child.wait_with_output(),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Agent '{}' timed out after {}s",
                agent.agent.id,
                timeout_secs
            )
        })?
        .context("Failed to collect claude output")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "claude exited with code {}: {}",
                output.status,
                stderr.trim()
            );
        }

        let text = String::from_utf8(output.stdout)
            .context("claude output is not valid UTF-8")?;

        if text.trim().is_empty() {
            anyhow::bail!(
                "claude returned an empty response for agent '{}'",
                agent.agent.id
            );
        }

        Ok(text)
    }

    fn parse_response(
        &self,
        agent: &AgentDefinition,
        response: &str,
    ) -> Result<Vec<GeneratedComment>> {
        // Strip markdown code fences if present
        let json_str = strip_markdown_fences(response);

        if agent.agent.id == "summary" {
            let raw: RawSummary = serde_json::from_str(json_str)
                .context("Failed to parse summary JSON")?;
            let severity = parse_severity(raw.severity.as_deref());
            let comment = GeneratedComment::new(
                CommentSource::Agent {
                    agent_id: agent.agent.id.clone(),
                    agent_name: agent.agent.name.clone(),
                    agent_icon: agent.agent.icon.clone(),
                },
                raw.body,
                severity,
                None,
                None,
            );
            Ok(vec![comment])
        } else {
            let raw_comments: Vec<RawComment> = serde_json::from_str(json_str)
                .context("Failed to parse comments JSON array")?;
            let comments = raw_comments
                .into_iter()
                .map(|rc| {
                    let severity = parse_severity(rc.severity.as_deref());
                    GeneratedComment::new(
                        CommentSource::Agent {
                            agent_id: agent.agent.id.clone(),
                            agent_name: agent.agent.name.clone(),
                            agent_icon: agent.agent.icon.clone(),
                        },
                        rc.body,
                        severity,
                        rc.file_path,
                        rc.line,
                    )
                })
                .collect();
            Ok(comments)
        }
    }
}

fn strip_markdown_fences(s: &str) -> &str {
    let s = s.trim();
    let s = if s.starts_with("```json") {
        s.trim_start_matches("```json")
    } else if s.starts_with("```") {
        s.trim_start_matches("```")
    } else {
        s
    };
    let s = if s.ends_with("```") {
        s.trim_end_matches("```")
    } else {
        s
    };
    s.trim()
}

fn parse_severity(s: Option<&str>) -> Severity {
    s.and_then(|v| v.parse::<Severity>().ok())
        .unwrap_or(Severity::Suggestion)
}
