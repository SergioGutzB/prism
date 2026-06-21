use std::time::Instant;

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::{debug, warn};

use crate::agents::context::{ObjectiveAlignment, ObjectiveAnalysis, ReviewContext};
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

/// Objective-validator agent (Phase 0) returns this shape.
/// Local models (Ollama) sometimes return `stated_objectives` / `implementation_summary`
/// as arrays instead of strings — `string_or_array` joins them with "; ".
#[derive(Debug, Deserialize)]
struct RawObjective {
    #[serde(deserialize_with = "string_or_array")]
    stated_objectives: String,
    #[serde(deserialize_with = "string_or_array")]
    implementation_summary: String,
    alignment: String,
    gaps: Option<Vec<String>>,
    #[serde(deserialize_with = "string_or_array")]
    overall_assessment: String,
}

/// Deserialize a JSON field that may be either a plain string or an array of strings.
/// Arrays are joined with "; ".
fn string_or_array<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    use serde::de::{self, Visitor};
    struct StrOrArr;
    impl<'de> Visitor<'de> for StrOrArr {
        type Value = String;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "a string or array of strings")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<String, E> {
            Ok(v.to_string())
        }
        fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<String, E> {
            Ok(v)
        }
        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<String, A::Error> {
            let mut parts = Vec::new();
            while let Some(s) = seq.next_element::<String>()? {
                parts.push(s);
            }
            Ok(parts.join("; "))
        }
    }
    d.deserialize_any(StrOrArr)
}

pub struct AgentRunner {
    config: AppConfig,
    /// Shared HTTP client for API-based providers.
    client: reqwest::Client,
}

impl AgentRunner {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// Execute a Phase-0 objective-validator agent.
    ///
    /// Returns the `AgentStatus` (for UI updates) **and** the structured
    /// `ObjectiveAnalysis` that will be injected into all later agents.
    pub async fn run_objective(
        &self,
        agent: &AgentDefinition,
        ctx: &ReviewContext,
    ) -> (AgentStatus, Option<ObjectiveAnalysis>) {
        if !agent.agent.enabled {
            return (AgentStatus::Disabled, None);
        }

        let start = Instant::now();
        match self.run_objective_inner(agent, ctx).await {
            Ok((analysis, comments, input_tokens, output_tokens)) => {
                let elapsed_ms = start.elapsed().as_millis() as u64;
                (
                    AgentStatus::Done { comments, elapsed_ms, input_tokens, output_tokens },
                    Some(analysis),
                )
            }
            Err(e) => (AgentStatus::Failed { error: format!("{:#}", e) }, None),
        }
    }

    async fn run_objective_inner(
        &self,
        agent: &AgentDefinition,
        ctx: &ReviewContext,
    ) -> Result<(ObjectiveAnalysis, Vec<GeneratedComment>, u64, u64)> {
        let prompt = self.build_prompt(agent, ctx);
        let system_len = agent.agent.prompt.system.len();
        debug!(agent_id = %agent.agent.id, prompt_len = prompt.len(), "Running Phase-0 objective agent");

        let raw_response = self.call_llm(agent, &prompt).await?;

        let input_tokens = ((system_len + prompt.len()) / 4) as u64;
        let output_tokens = (raw_response.len() / 4) as u64;

        let json_str = clean_llm_response(&raw_response);
        let raw: RawObjective = match serde_json::from_str(&json_str) {
            Ok(r) => r,
            Err(e) => {
                // JSON parse failed — local models (Ollama) often truncate or mis-format.
                // Log the raw response for debugging, then return a degraded result
                // so the other agents can still run with a neutral ObjectiveAnalysis.
                warn!(
                    agent_id = %agent.agent.id,
                    error = %e,
                    raw_len = raw_response.len(),
                    raw_preview = %&raw_response.chars().take(200).collect::<String>(),
                    "Objective JSON parse failed — using degraded fallback"
                );
                RawObjective {
                    stated_objectives: "Could not parse objectives (JSON parse error)".into(),
                    implementation_summary: raw_response.chars().take(500).collect(),
                    alignment: "partial".into(),
                    gaps: None,
                    overall_assessment: format!(
                        "Objective Validator could not parse the model response as JSON ({e}). \
                        Review the diff manually for objective alignment."
                    ),
                }
            }
        };

        let alignment = match raw.alignment.to_lowercase().trim() {
            "aligned"    => ObjectiveAlignment::Aligned,
            "misaligned" => ObjectiveAlignment::Misaligned,
            _            => ObjectiveAlignment::Partial,
        };

        let analysis = ObjectiveAnalysis {
            stated_objectives: raw.stated_objectives,
            implementation_summary: raw.implementation_summary,
            alignment,
            gaps: raw.gaps.unwrap_or_default(),
            overall_assessment: raw.overall_assessment.clone(),
        };

        // Generate a single comment for UI display
        let sev = match analysis.alignment {
            ObjectiveAlignment::Misaligned => Severity::Warning,
            ObjectiveAlignment::Partial    => Severity::Suggestion,
            ObjectiveAlignment::Aligned    => Severity::Praise,
        };
        let comment = GeneratedComment::new(
            CommentSource::Agent {
                agent_id: agent.agent.id.clone(),
                agent_name: agent.agent.name.clone(),
                agent_icon: agent.agent.icon.clone(),
            },
            raw.overall_assessment,
            sev,
            None,
            None,
        );

        Ok((analysis, vec![comment], input_tokens, output_tokens))
    }

    /// Execute a single agent against the given review context.
    pub async fn run(&self, agent: &AgentDefinition, ctx: &ReviewContext) -> AgentStatus {
        if !agent.agent.enabled {
            return AgentStatus::Disabled;
        }

        let start = Instant::now();
        match self.run_inner(agent, ctx).await {
            Ok((comments, input_tokens, output_tokens)) => {
                let elapsed_ms = start.elapsed().as_millis() as u64;
                AgentStatus::Done { comments, elapsed_ms, input_tokens, output_tokens }
            }
            Err(e) => AgentStatus::Failed { error: format!("{:#}", e) },
        }
    }

    async fn run_inner(
        &self,
        agent: &AgentDefinition,
        ctx: &ReviewContext,
    ) -> Result<(Vec<GeneratedComment>, u64, u64)> {
        let prompt = self.build_prompt(agent, ctx);
        let system_len = agent.agent.prompt.system.len();
        debug!(agent_id = %agent.agent.id, prompt_len = prompt.len(), provider = %self.config.llm.provider, "Running agent");

        let raw_response = self.call_llm(agent, &prompt).await?;

        let input_tokens = ((system_len + prompt.len()) / 4) as u64;
        let output_tokens = (raw_response.len() / 4) as u64;

        match self.parse_response(agent, &raw_response) {
            Ok(comments) => {
                let filtered = self.apply_severity_filter(agent, comments);
                Ok((filtered, input_tokens, output_tokens))
            }
            Err(parse_err) => {
                warn!(agent_id = %agent.agent.id, "First parse attempt failed ({}), retrying", parse_err);
                // Include the expected schema in the retry to help models that drifted
                let schema_hint = if agent.agent.id == "objective" {
                    r#"Respond ONLY with this exact JSON object (no prose, no fences, no <think> blocks):
{"stated_objectives":"...","implementation_summary":"...","alignment":"aligned|partial|misaligned","gaps":[],"overall_assessment":"..."}"#
                } else if agent.agent.id == "summary" {
                    r#"Respond ONLY with this exact JSON object (no prose, no fences, no <think> blocks):
{"body":"...","severity":"suggestion|warning|critical|praise"}"#
                } else {
                    r#"Respond ONLY with a JSON array (no prose, no fences, no <think> blocks):
[{"file_path":"path/to/file.rs","line":42,"severity":"suggestion|warning|critical|praise","body":"comment text"}]
Use null for file_path and line when the comment is general."#
                };
                let retry_prompt = format!(
                    "{}\n\n---\nYour previous response could not be parsed as JSON (error: {}).\n{}",
                    prompt, parse_err, schema_hint
                );
                let retry_response = self.call_llm(agent, &retry_prompt).await?;
                let retry_in = (retry_prompt.len() / 4) as u64;
                let retry_out = (retry_response.len() / 4) as u64;
                let comments = self.parse_response(agent, &retry_response)
                    .context("JSON parse failed after retry")?;
                let filtered = self.apply_severity_filter(agent, comments);
                Ok((filtered, input_tokens + retry_in, output_tokens + retry_out))
            }
        }
    }

    /// Post-generation safety net: discard comments that fall below the effective
    /// minimum severity for this agent. The LLM rigor instruction is the primary
    /// gate; this filter catches the cases where the model ignores it.
    ///
    /// The effective threshold is resolved as:
    ///   agent `min_severity` (frontmatter) > global `review_rigor` > no filter
    ///
    /// Summary and objective agents are exempt — their comments carry structural
    /// meaning (overall verdict) that should never be filtered by rigor level.
    fn apply_severity_filter(
        &self,
        agent: &AgentDefinition,
        comments: Vec<GeneratedComment>,
    ) -> Vec<GeneratedComment> {
        // Exempt structural agents from severity filtering
        if matches!(agent.agent.id.as_str(), "summary" | "objective") {
            return comments;
        }

        let global_min = self.config.agents.min_severity_for_rigor();
        let min = agent.agent.effective_min_severity(global_min.as_ref());

        match min {
            None => comments,
            Some(threshold) => {
                let before = comments.len();
                let filtered: Vec<_> = comments
                    .into_iter()
                    .filter(|c| c.severity >= threshold)
                    .collect();
                let dropped = before - filtered.len();
                if dropped > 0 {
                    debug!(
                        agent_id = %agent.agent.id,
                        dropped,
                        threshold = %threshold,
                        "Rigor filter dropped comments below severity threshold",
                    );
                }
                filtered
            }
        }
    }

    fn build_prompt(&self, agent: &AgentDefinition, ctx: &ReviewContext) -> String {
        let mut parts: Vec<String> = Vec::new();

        // Repository context: language, frameworks, conventions — always injected
        {
            let mut repo_ctx = String::from("## Repository Context\n\n");
            if let Some(lang) = &ctx.repo_language {
                repo_ctx.push_str(&format!("**Primary Language:** {lang}\n"));
            }
            if !ctx.detected_frameworks.is_empty() {
                repo_ctx.push_str(&format!(
                    "**Detected Frameworks/Tools:** {}\n",
                    ctx.detected_frameworks.join(", ")
                ));
            }
            if let Some(conventions) = &ctx.project_conventions {
                let truncated = if conventions.len() > 2000 {
                    format!("{}…\n*(truncated)*", &conventions[..2000])
                } else {
                    conventions.clone()
                };
                repo_ctx.push_str(&format!("\n**Project Conventions:**\n{truncated}\n"));
            }
            // Only push if we have something beyond the header
            if repo_ctx.len() > "## Repository Context\n\n".len() {
                parts.push(repo_ctx);
            }
        }

        if agent.agent.context.include_pr_description {
            parts.push(format!(
                "## PR: {} (#{}) by {}\n\n{}",
                ctx.pr_title, ctx.pr_number, ctx.pr_author, ctx.pr_description
            ));
        }
        // Inject Phase-0 objective analysis so every later agent knows the
        // stated objectives and alignment verdict.
        if let Some(obj_text) = ctx.objective_text() {
            parts.push(obj_text);
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
        // Phase 2 synthesis agents receive the aggregated specialist findings
        if let Some(findings_md) = ctx.findings_text() {
            parts.push(format!("## Team Findings\n\n{}", findings_md));
        }
        if agent.agent.context.include_diff {
            let prepared = ctx.prepare_diff(
                &self.config.agents.diff_exclude_patterns,
                &agent.agent.context.exclude_patterns,
                &agent.agent.context.include_patterns,
                self.config.agents.max_diff_tokens,
            );

            debug!(
                agent_id = %agent.agent.id,
                files_included = prepared.files_included,
                files_excluded = prepared.files_excluded,
                files_truncated = prepared.files_truncated,
                est_tokens = prepared.estimated_tokens(),
                "Diff prepared for agent",
            );

            let diff_section = if prepared.diff.is_empty() {
                // Everything was filtered — tell the LLM so it doesn't hallucinate
                let note = prepared.header_note()
                    .unwrap_or_else(|| "all files were excluded".to_string());
                format!("## Diff\n\n> Note: {note}\n\n(No diff content — all changed files matched exclusion patterns.)")
            } else {
                match prepared.header_note() {
                    Some(note) => format!(
                        "## Diff\n> Note: {note}\n\n```diff\n{}\n```",
                        prepared.diff
                    ),
                    None => format!("## Diff\n\n```diff\n{}\n```", prepared.diff),
                }
            };

            parts.push(diff_section);
        }

        // Inject the rigor instruction immediately before the prompt suffix so the
        // model reads it as the last constraint before generating output — the most
        // influential position. Agents with `min_severity` set in their frontmatter
        // skip the global rigor instruction; the post-generation filter still applies.
        if agent.agent.min_severity.is_none() {
            if let Some(instruction) = self.config.agents.rigor_instruction() {
                parts.push(instruction.to_string());
            }
        }

        parts.push(agent.agent.prompt.prompt_suffix.clone());
        parts.join("\n\n")
    }

    /// Dispatch to the right LLM backend based on `config.llm.provider`.
    async fn call_llm(&self, agent: &AgentDefinition, prompt: &str) -> Result<String> {
        let system = agent.agent.prompt.system.clone();

        // Per-agent model/temperature override (if defined in agent YAML)
        let model = agent.agent.llm.as_ref()
            .and_then(|o| o.model.as_deref())
            .unwrap_or(&self.config.llm.model);
        let temperature = agent.agent.llm.as_ref()
            .and_then(|o| o.temperature)
            .unwrap_or(self.config.llm.temperature);
        let max_tokens = agent.agent.llm.as_ref()
            .and_then(|o| o.max_tokens)
            .unwrap_or(self.config.llm.max_tokens);

        let timeout = self.config.agents.timeout_secs;

        call_provider(
            &self.client,
            &self.config.llm,
            model,
            temperature,
            max_tokens,
            &system,
            prompt,
            timeout,
            &agent.agent.id,
        )
        .await
    }

    fn parse_response(&self, agent: &AgentDefinition, response: &str) -> Result<Vec<GeneratedComment>> {
        let json_str = clean_llm_response(response);

        if agent.agent.id == "summary" {
            let raw: RawSummary = serde_json::from_str(&json_str)
                .context("Failed to parse summary JSON")?;
            let comment = GeneratedComment::new(
                CommentSource::Agent {
                    agent_id: agent.agent.id.clone(),
                    agent_name: agent.agent.name.clone(),
                    agent_icon: agent.agent.icon.clone(),
                },
                raw.body,
                parse_severity(raw.severity.as_deref()),
                None,
                None,
            );
            Ok(vec![comment])
        } else {
            let raw_comments: Vec<RawComment> = serde_json::from_str(&json_str)
                .context("Failed to parse comments JSON array")?;
            Ok(raw_comments
                .into_iter()
                .map(|rc| {
                    GeneratedComment::new(
                        CommentSource::Agent {
                            agent_id: agent.agent.id.clone(),
                            agent_name: agent.agent.name.clone(),
                            agent_icon: agent.agent.icon.clone(),
                        },
                        rc.body,
                        parse_severity(rc.severity.as_deref()),
                        rc.file_path,
                        rc.line,
                    )
                })
                .collect())
        }
    }
}

// ── Multi-provider dispatch ────────────────────────────────────────────────────

/// Call the configured LLM provider and return the raw text response.
/// This is a free function so it can be reused from main.rs (e.g. ClaudeCodeFix).
#[allow(clippy::too_many_arguments)]
pub async fn call_provider(
    client: &reqwest::Client,
    llm: &crate::config::LlmConfig,
    model: &str,
    temperature: f32,
    max_tokens: u32,
    system: &str,
    prompt: &str,
    timeout_secs: u64,
    context_label: &str,
) -> Result<String> {
    match llm.provider.as_str() {
        "claude-cli" | "claude" => {
            call_claude_cli(system, prompt, timeout_secs, context_label).await
        }
        "anthropic" => {
            call_anthropic(client, llm, model, temperature, max_tokens, system, prompt, timeout_secs).await
        }
        "openai" | "codex" => {
            call_openai(client, llm, model, temperature, max_tokens, system, prompt, timeout_secs).await
        }
        "gemini" => {
            call_gemini(client, llm, model, temperature, max_tokens, system, prompt, timeout_secs).await
        }
        "ollama" => {
            call_ollama(client, llm, model, temperature, max_tokens, system, prompt, timeout_secs).await
        }
        other => anyhow::bail!(
            "Unknown LLM provider: '{}'. Supported: claude-cli, anthropic, openai, gemini, ollama",
            other
        ),
    }
}

// ── claude-cli ────────────────────────────────────────────────────────────────

async fn call_claude_cli(
    system: &str,
    prompt: &str,
    timeout_secs: u64,
    context_label: &str,
) -> Result<String> {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    let mut child = Command::new("claude")
        .arg("--print")
        .arg("--system-prompt")
        .arg(system)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("Failed to spawn `claude` — is Claude Code installed and in PATH?")?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).await
            .context("Failed to write prompt to claude stdin")?;
    }

    // kill_on_drop(true) ensures the child is killed if the timeout fires and drops the future
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("'{}' timed out after {}s (claude-cli)", context_label, timeout_secs))?
    .context("Failed to collect claude output")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("claude exited with {}: {}", output.status, stderr.trim());
    }
    let text = String::from_utf8(output.stdout).context("claude output is not valid UTF-8")?;
    if text.trim().is_empty() {
        anyhow::bail!("claude returned an empty response for '{}'", context_label);
    }
    Ok(text)
}

// ── Anthropic Messages API ────────────────────────────────────────────────────

async fn call_anthropic(
    client: &reqwest::Client,
    llm: &crate::config::LlmConfig,
    model: &str,
    temperature: f32,
    max_tokens: u32,
    system: &str,
    prompt: &str,
    timeout_secs: u64,
) -> Result<String> {
    let api_key = llm.effective_api_key();
    if api_key.is_empty() {
        anyhow::bail!("Anthropic API key not set — set ANTHROPIC_API_KEY or llm.api_key in config");
    }
    let base = llm.effective_base_url();
    let url = format!("{}/v1/messages", base);

    let body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "system": system,
        "messages": [{"role": "user", "content": prompt}]
    });

    debug!("POST Anthropic /v1/messages model={}", model);

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        client
            .post(&url)
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Anthropic API timed out after {}s", timeout_secs))?
    .context("Failed to reach Anthropic API")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic API error {}: {}", status, text);
    }

    let json: serde_json::Value = resp.json().await.context("Failed to parse Anthropic response")?;
    extract_text(&json, &["content", "0", "text"])
        .ok_or_else(|| anyhow::anyhow!("Unexpected Anthropic response shape: {}", json))
}

// ── OpenAI Chat Completions API ───────────────────────────────────────────────

async fn call_openai(
    client: &reqwest::Client,
    llm: &crate::config::LlmConfig,
    model: &str,
    temperature: f32,
    max_tokens: u32,
    system: &str,
    prompt: &str,
    timeout_secs: u64,
) -> Result<String> {
    let api_key = llm.effective_api_key();
    if api_key.is_empty() {
        anyhow::bail!("OpenAI API key not set — set OPENAI_API_KEY or llm.api_key in config");
    }
    let base = llm.effective_base_url();
    let url = format!("{}/v1/chat/completions", base);

    let body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user",   "content": prompt}
        ]
    });

    debug!("POST OpenAI /v1/chat/completions model={}", model);

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        client
            .post(&url)
            .bearer_auth(&api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("OpenAI API timed out after {}s", timeout_secs))?
    .context("Failed to reach OpenAI API")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("OpenAI API error {}: {}", status, text);
    }

    let json: serde_json::Value = resp.json().await.context("Failed to parse OpenAI response")?;
    extract_text(&json, &["choices", "0", "message", "content"])
        .ok_or_else(|| anyhow::anyhow!("Unexpected OpenAI response shape: {}", json))
}

// ── Google Gemini API ─────────────────────────────────────────────────────────

async fn call_gemini(
    client: &reqwest::Client,
    llm: &crate::config::LlmConfig,
    model: &str,
    temperature: f32,
    max_tokens: u32,
    system: &str,
    prompt: &str,
    timeout_secs: u64,
) -> Result<String> {
    let api_key = llm.effective_api_key();
    if api_key.is_empty() {
        anyhow::bail!("Gemini API key not set — set GEMINI_API_KEY (or GOOGLE_API_KEY) or llm.api_key in config");
    }
    let base = llm.effective_base_url();
    let url = format!(
        "{}/v1beta/models/{}:generateContent?key={}",
        base, model, api_key
    );

    // Merge system instructions into the prompt for maximum compatibility.
    // This avoids all versioning issues with the 'systemInstruction' field.
    let combined_prompt = if system.is_empty() {
        prompt.to_string()
    } else {
        format!("SYSTEM INSTRUCTIONS:\n{}\n\nUSER PROMPT:\n{}", system, prompt)
    };

    let body = serde_json::json!({
        "contents": [{
            "role": "user",
            "parts": [{"text": combined_prompt}]
        }],
        "generationConfig": {
            "maxOutputTokens": max_tokens,
            "temperature": temperature
        }
    });

    let mut attempts = 0;
    let max_attempts = 3;

    loop {
        attempts += 1;
        debug!("POST Gemini generateContent model={} attempt={}", model, attempts);

        let resp = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            client
                .post(&url)
                .header("content-type", "application/json")
                .json(&body)
                .send(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Gemini API timed out after {}s", timeout_secs))?
        .context("Failed to reach Gemini API")?;

        if resp.status().is_success() {
            let json: serde_json::Value = resp.json().await.context("Failed to parse Gemini response")?;
            return extract_text(&json, &["candidates", "0", "content", "parts", "0", "text"])
                .ok_or_else(|| anyhow::anyhow!("Unexpected Gemini response shape: {}", json));
        }

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if status.as_u16() == 429 && attempts < max_attempts {
            warn!("Gemini API rate limited (429), retrying in 2s... (attempt {}/{})", attempts, max_attempts);
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        }

        // Improved error message for debugging
        anyhow::bail!("Gemini API error {}: {}", status, text);
    }
}

// ── Ollama (local) ────────────────────────────────────────────────────────────

async fn call_ollama(
    client: &reqwest::Client,
    llm: &crate::config::LlmConfig,
    model: &str,
    temperature: f32,
    max_tokens: u32,
    system: &str,
    prompt: &str,
    timeout_secs: u64,
) -> Result<String> {
    let base = llm.effective_base_url();
    let url = format!("{}/api/chat", base);

    // Build options — always set temperature and output budget.
    // If ollama_num_ctx > 0, request that context window explicitly so Ollama
    // doesn't silently truncate long prompts (e.g. "truncating input prompt" in logs).
    let mut options = serde_json::json!({
        "temperature": temperature,
        "num_predict": max_tokens,
    });
    if llm.ollama_num_ctx > 0 {
        options["num_ctx"] = serde_json::json!(llm.ollama_num_ctx);
    }

    let body = serde_json::json!({
        "model": model,
        "stream": false,
        "messages": [
            {"role": "system",  "content": system},
            {"role": "user",    "content": prompt}
        ],
        "options": options
    });

    debug!("POST Ollama /api/chat model={}", model);

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        client
            .post(&url)
            .header("content-type", "application/json")
            .json(&body)
            .send(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Ollama timed out after {}s — is it running?", timeout_secs))?
    .context("Failed to reach Ollama (is it running on localhost:11434?)")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Ollama error {}: {}", status, text);
    }

    let json: serde_json::Value = resp.json().await.context("Failed to parse Ollama response")?;
    extract_text(&json, &["message", "content"])
        .ok_or_else(|| anyhow::anyhow!("Unexpected Ollama response shape: {}", json))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Navigate a JSON value using a path of string keys/array indices.
fn extract_text(v: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut cur = v;
    for key in path {
        cur = match cur {
            serde_json::Value::Object(m) => m.get(*key)?,
            serde_json::Value::Array(a) => {
                let idx: usize = key.parse().ok()?;
                a.get(idx)?
            }
            _ => return None,
        };
    }
    cur.as_str().map(|s| s.to_string())
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
    let s = if s.ends_with("```") { s.trim_end_matches("```") } else { s };
    s.trim()
}

/// Strip `<think>…</think>` blocks emitted by reasoning models (Qwen, DeepSeek, etc.)
/// before the actual response. Returns a slice starting after the last closing tag,
/// or the original string if no such tags are found.
fn strip_thinking_blocks(s: &str) -> &str {
    // Tags used by various models: <think>, <|thinking|>, <reasoning>, <internal>
    const CLOSE_TAGS: &[&str] = &["</think>", "</|thinking|>", "</reasoning>", "</internal>"];
    let mut best_end = 0usize;
    for tag in CLOSE_TAGS {
        if let Some(pos) = s.rfind(tag) {
            let end = pos + tag.len();
            if end > best_end {
                best_end = end;
            }
        }
    }
    if best_end > 0 {
        s[best_end..].trim()
    } else {
        s
    }
}

/// Remove trailing commas before `}` or `]` — common in LLM-generated JSON.
/// Also attempts to close truncated arrays/objects by counting unclosed brackets.
fn repair_json(s: &str) -> std::borrow::Cow<'_, str> {
    // Fast path: if it already parses, don't touch it
    if serde_json::from_str::<serde_json::Value>(s).is_ok() {
        return std::borrow::Cow::Borrowed(s);
    }

    // Remove trailing commas before closing bracket/brace
    let mut result = String::with_capacity(s.len() + 8);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for a comma followed only by whitespace then } or ]
        if bytes[i] == b',' {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n' || bytes[j] == b'\r') {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'}' || bytes[j] == b']') {
                i += 1; // skip the comma
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }

    // If the JSON is truncated, try to close open brackets/braces
    let mut depth_obj: i32 = 0;
    let mut depth_arr: i32 = 0;
    let mut in_string = false;
    let mut escape_next = false;
    for ch in result.chars() {
        if escape_next { escape_next = false; continue; }
        if ch == '\\' && in_string { escape_next = true; continue; }
        if ch == '"' { in_string = !in_string; continue; }
        if in_string { continue; }
        match ch {
            '{' => depth_obj += 1,
            '}' => depth_obj -= 1,
            '[' => depth_arr += 1,
            ']' => depth_arr -= 1,
            _ => {}
        }
    }
    // Close any unclosed string first, then brackets
    if in_string { result.push('"'); }
    for _ in 0..depth_arr.max(0) { result.push(']'); }
    for _ in 0..depth_obj.max(0) { result.push('}'); }

    std::borrow::Cow::Owned(result)
}

/// After stripping fences, find the first `{` or `[` and last `}` or `]` to extract
/// valid JSON even when the LLM prepends preamble text before the JSON object.
fn extract_json(s: &str) -> &str {
    let start = s.find(|c| c == '{' || c == '[');
    let end_obj = s.rfind('}');
    let end_arr = s.rfind(']');
    let end = match (end_obj, end_arr) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    match (start, end) {
        (Some(i), Some(j)) if j > i => &s[i..=j],
        _ => s,
    }
}

/// Full pipeline: strip thinking blocks → strip fences → extract JSON bounds → repair.
/// Returns an owned String so callers can pass it to serde_json.
fn clean_llm_response(raw: &str) -> String {
    let s = strip_thinking_blocks(raw);
    let s = strip_markdown_fences(s);
    let s = extract_json(s);
    repair_json(s).into_owned()
}

fn parse_severity(s: Option<&str>) -> Severity {
    s.and_then(|v| v.parse::<Severity>().ok())
        .unwrap_or(Severity::Suggestion)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── strip_markdown_fences ─────────────────────────────────────────────────

    #[test]
    fn strip_fences_removes_json_tagged_block() {
        let input = "```json\n[{\"body\":\"ok\"}]\n```";
        assert_eq!(strip_markdown_fences(input), "[{\"body\":\"ok\"}]");
    }

    #[test]
    fn strip_fences_removes_untagged_block() {
        let input = "```\n{\"key\":\"val\"}\n```";
        assert_eq!(strip_markdown_fences(input), "{\"key\":\"val\"}");
    }

    #[test]
    fn strip_fences_no_fences_unchanged() {
        let input = "[{\"body\":\"hello\"}]";
        assert_eq!(strip_markdown_fences(input), input);
    }

    #[test]
    fn strip_fences_trims_whitespace() {
        let input = "  \n```json\n{}\n```\n  ";
        assert_eq!(strip_markdown_fences(input), "{}");
    }

    #[test]
    fn strip_fences_only_opening_fence_returned_as_content() {
        // No closing fence — trim_end_matches won't strip anything, content is returned as-is
        let input = "```json\n[1, 2, 3]";
        let result = strip_markdown_fences(input);
        // Should still return the content (minus opening fence), not crash
        assert!(result.contains("[1, 2, 3]"));
    }

    // ── extract_json ──────────────────────────────────────────────────────────

    #[test]
    fn extract_json_clean_object() {
        let input = r#"{"body":"hello"}"#;
        assert_eq!(extract_json(input), r#"{"body":"hello"}"#);
    }

    #[test]
    fn extract_json_clean_array() {
        let input = r#"[{"body":"a"},{"body":"b"}]"#;
        assert_eq!(extract_json(input), input);
    }

    #[test]
    fn extract_json_strips_preamble() {
        let input = r#"Here is the JSON output: {"body":"x","severity":"warning"}"#;
        assert_eq!(extract_json(input), r#"{"body":"x","severity":"warning"}"#);
    }

    #[test]
    fn extract_json_strips_postamble() {
        let input = r#"{"body":"x"} Hope this helps!"#;
        // rfind('}') finds the one after "x", rfind(']') finds nothing → slice [0..=9]
        assert_eq!(extract_json(input), r#"{"body":"x"}"#);
    }

    #[test]
    fn extract_json_strips_preamble_and_postamble() {
        let input = r#"Sure! Here: [{"body":"y"}] Done."#;
        assert_eq!(extract_json(input), r#"[{"body":"y"}]"#);
    }

    #[test]
    fn extract_json_nested_objects_preserved() {
        let input = r#"{"outer":{"inner":"val"}}"#;
        assert_eq!(extract_json(input), input);
    }

    #[test]
    fn extract_json_no_json_returns_original() {
        let input = "No JSON here at all";
        assert_eq!(extract_json(input), input);
    }

    #[test]
    fn extract_json_only_open_brace_returns_original() {
        // start found but no matching end → returns original
        let input = "prefix { no close";
        // start=7, no '}' → returns original
        assert_eq!(extract_json(input), input);
    }

    #[test]
    fn extract_json_array_inside_prose() {
        let input = "Result: [{\"file_path\":\"foo.rs\",\"line\":1,\"body\":\"b\",\"severity\":\"warning\"}]";
        let result = extract_json(input);
        assert!(result.starts_with('['));
        assert!(result.ends_with(']'));
        assert!(result.contains("\"body\":\"b\""));
    }

    // ── parse_severity ────────────────────────────────────────────────────────

    #[test]
    fn parse_severity_known_values() {
        assert_eq!(parse_severity(Some("critical")), Severity::Critical);
        assert_eq!(parse_severity(Some("warning")), Severity::Warning);
        assert_eq!(parse_severity(Some("suggestion")), Severity::Suggestion);
        assert_eq!(parse_severity(Some("praise")), Severity::Praise);
    }

    #[test]
    fn parse_severity_unknown_defaults_to_suggestion() {
        assert_eq!(parse_severity(Some("blocker")), Severity::Suggestion);
        assert_eq!(parse_severity(None), Severity::Suggestion);
    }

    #[test]
    fn parse_severity_case_insensitive() {
        assert_eq!(parse_severity(Some("CRITICAL")), Severity::Critical);
        assert_eq!(parse_severity(Some("Warning")), Severity::Warning);
    }
}
