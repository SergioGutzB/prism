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
    },
    Failed {
        error: String,
    },
    Skipped {
        reason: String,
    },
}
