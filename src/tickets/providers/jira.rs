use async_trait::async_trait;
use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::debug;

use crate::tickets::models::Ticket;
use crate::tickets::provider::TicketProvider;

pub struct JiraProvider {
    client: reqwest::Client,
    base_url: String,
    email: String,
    api_token: String,
    key_patterns: Vec<String>,
}

impl JiraProvider {
    pub fn new(
        base_url: &str,
        email: &str,
        api_token: &str,
        key_patterns: Vec<String>,
    ) -> Result<Self> {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .build()
            .context("Failed to build HTTP client")?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            email: email.to_string(),
            api_token: api_token.to_string(),
            key_patterns,
        })
    }
}

#[async_trait]
impl TicketProvider for JiraProvider {
    fn name(&self) -> &str {
        "jira"
    }

    fn key_patterns(&self) -> &[String] {
        &self.key_patterns
    }

    async fn get_ticket(&self, key: &str) -> Result<Option<Ticket>> {
        if self.base_url.is_empty() || self.email.is_empty() || self.api_token.is_empty() {
            return Ok(None);
        }

        let url = format!("{}/rest/api/3/issue/{}", self.base_url, key);
        debug!("Jira GET {}", url);

        let response = self
            .client
            .get(&url)
            .basic_auth(&self.email, Some(&self.api_token))
            .header("Accept", "application/json")
            .send()
            .await
            .context("Jira request failed")?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Jira API error {}: {}", status, body);
        }

        let issue: JiraIssue = response.json().await.context("Failed to parse Jira issue")?;

        let description = extract_jira_description(&issue.fields.description);

        Ok(Some(Ticket {
            key: issue.key.clone(),
            title: issue.fields.summary,
            description,
            status: issue
                .fields
                .status
                .map(|s| s.name)
                .unwrap_or_else(|| "Unknown".to_string()),
            ticket_type: issue
                .fields
                .issuetype
                .map(|t| t.name)
                .unwrap_or_else(|| "Task".to_string()),
            priority: issue.fields.priority.map(|p| p.name),
            assignee: issue.fields.assignee.map(|a| a.display_name),
            reporter: issue.fields.reporter.map(|r| r.display_name),
            labels: issue.fields.labels.unwrap_or_default(),
            url: format!("{}/browse/{}", self.base_url, issue.key),
            provider: "jira".to_string(),
            created_at: issue.fields.created,
            updated_at: issue.fields.updated,
        }))
    }

    async fn is_available(&self) -> bool {
        if self.base_url.is_empty() {
            return false;
        }
        let url = format!("{}/rest/api/3/myself", self.base_url);
        match self
            .client
            .get(&url)
            .basic_auth(&self.email, Some(&self.api_token))
            .send()
            .await
        {
            Ok(r) => r.status().is_success(),
            Err(_) => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Jira API response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JiraIssue {
    pub key: String,
    pub fields: JiraFields,
}

#[derive(Debug, Deserialize)]
struct JiraFields {
    pub summary: String,
    pub description: Option<serde_json::Value>,
    pub status: Option<JiraStatus>,
    pub issuetype: Option<JiraIssueType>,
    pub priority: Option<JiraPriority>,
    pub assignee: Option<JiraUser>,
    pub reporter: Option<JiraUser>,
    pub labels: Option<Vec<String>>,
    pub created: Option<chrono::DateTime<chrono::Utc>>,
    pub updated: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
struct JiraStatus {
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct JiraIssueType {
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct JiraPriority {
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct JiraUser {
    #[serde(rename = "displayName")]
    pub display_name: String,
}

/// Extract plain text from Jira's Atlassian Document Format (ADF).
fn extract_jira_description(value: &Option<serde_json::Value>) -> Option<String> {
    let v = value.as_ref()?;

    // If it's a plain string (older Jira API)
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }

    // Walk ADF nodes
    let mut text = String::new();
    collect_adf_text(v, &mut text);
    if text.is_empty() {
        None
    } else {
        Some(text.trim().to_string())
    }
}

fn collect_adf_text(node: &serde_json::Value, out: &mut String) {
    if let Some(t) = node.get("text").and_then(|v| v.as_str()) {
        out.push_str(t);
    }
    if let Some(content) = node.get("content").and_then(|v| v.as_array()) {
        for child in content {
            collect_adf_text(child, out);
        }
        out.push('\n');
    }
}
