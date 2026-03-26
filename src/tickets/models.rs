use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    pub key: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub ticket_type: String,
    pub priority: Option<String>,
    pub assignee: Option<String>,
    pub reporter: Option<String>,
    pub labels: Vec<String>,
    pub url: String,
    pub provider: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl Ticket {
    /// Format the ticket as a compact text block for LLM context.
    pub fn as_context_text(&self) -> String {
        let mut parts = vec![
            format!("Ticket: {} — {}", self.key, self.title),
            format!("Status: {} | Type: {}", self.status, self.ticket_type),
        ];
        if let Some(priority) = &self.priority {
            parts.push(format!("Priority: {}", priority));
        }
        if let Some(desc) = &self.description {
            parts.push(format!("\nDescription:\n{}", desc));
        }
        if !self.labels.is_empty() {
            parts.push(format!("Labels: {}", self.labels.join(", ")));
        }
        parts.join("\n")
    }
}
