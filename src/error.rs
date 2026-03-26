use thiserror::Error;

#[derive(Debug, Error)]
pub enum PrismError {
    #[error("Configuration error: {0}")]
    Config(#[from] config::ConfigError),

    #[error("GitHub API error: {0}")]
    GitHub(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML deserialization error: {0}")]
    TomlDe(#[from] toml::de::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Ticket provider error: {0}")]
    Ticket(String),

    #[error("Agent error (agent={agent_id}): {message}")]
    Agent { agent_id: String, message: String },

    #[error("LLM error: {0}")]
    Llm(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Not configured: {0}")]
    NotConfigured(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, PrismError>;
