use serde::{Deserialize, Serialize};
use crate::review::models::GeneratedComment;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub agent: AgentMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMeta {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub order: u32,
    pub icon: String,
    pub color: String,
    pub llm: Option<AgentLlmOverride>,
    pub prompt: AgentPrompt,
    pub context: AgentContext,
    /// Phase 0: runs BEFORE specialist agents. Receives full context but no
    /// prior findings. Its output (ObjectiveAnalysis) is injected into all
    /// later agents. Use for objective-validation agents. Default: false.
    #[serde(default)]
    pub phase_zero: bool,
    /// Phase 2: runs AFTER all specialist agents and receives their aggregated
    /// findings as additional context. Use for synthesis/summary agents. Default: false.
    #[serde(default)]
    pub synthesis: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLlmOverride {
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPrompt {
    pub system: String,
    pub prompt_suffix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentContext {
    pub include_diff: bool,
    pub include_pr_description: bool,
    pub include_ticket: bool,
    pub include_file_list: bool,
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
    #[serde(default)]
    pub include_patterns: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum AgentStatus {
    Disabled,
    Pending,
    Running {
        started_at: chrono::DateTime<chrono::Utc>,
    },
    Done {
        comments: Vec<GeneratedComment>,
        elapsed_ms: u64,
        input_tokens: u64,
        output_tokens: u64,
    },
    Failed {
        error: String,
    },
    Skipped {
        reason: String,
    },
}
