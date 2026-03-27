use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct GeneratedComment {
    pub id: uuid::Uuid,
    pub source: CommentSource,
    pub file_path: Option<String>,
    pub line: Option<u32>,
    pub body: String,
    pub edited_body: Option<String>,
    pub severity: Severity,
    pub status: CommentStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl GeneratedComment {
    pub fn new(
        source: CommentSource,
        body: String,
        severity: Severity,
        file_path: Option<String>,
        line: Option<u32>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            source,
            file_path,
            line,
            body,
            edited_body: None,
            severity,
            status: CommentStatus::Pending,
            created_at: chrono::Utc::now(),
        }
    }

    pub fn effective_body(&self) -> &str {
        self.edited_body.as_deref().unwrap_or(&self.body)
    }
}

#[derive(Debug, Clone)]
pub enum CommentSource {
    Agent {
        agent_id: String,
        agent_name: String,
        agent_icon: String,
    },
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    Praise,
    Suggestion,
    Warning,
    Critical,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Praise => write!(f, "praise"),
            Severity::Suggestion => write!(f, "suggestion"),
            Severity::Warning => write!(f, "warning"),
            Severity::Critical => write!(f, "critical"),
        }
    }
}

impl std::str::FromStr for Severity {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "praise" => Ok(Severity::Praise),
            "suggestion" => Ok(Severity::Suggestion),
            "warning" => Ok(Severity::Warning),
            "critical" => Ok(Severity::Critical),
            other => Err(anyhow::anyhow!("Unknown severity: {}", other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommentStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReviewEvent {
    Approve,
    Comment,
    RequestChanges,
}

impl ReviewEvent {
    pub fn as_github_str(&self) -> &str {
        match self {
            ReviewEvent::Approve => "APPROVE",
            ReviewEvent::Comment => "COMMENT",
            ReviewEvent::RequestChanges => "REQUEST_CHANGES",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReviewMode {
    AiOnly,
    ManualOnly,
    Hybrid,
}

#[derive(Debug, Clone)]
pub struct ReviewDraft {
    pub pr_number: u64,
    pub comments: Vec<GeneratedComment>,
    pub review_body: Option<String>,
    pub review_event: ReviewEvent,
    pub file_checklist: IndexMap<String, bool>,
    pub mode: ReviewMode,
    /// When the review session was first started (for resume detection).
    pub started_at: chrono::DateTime<chrono::Utc>,
}

impl ReviewDraft {
    pub fn new(pr_number: u64, mode: ReviewMode) -> Self {
        Self {
            pr_number,
            comments: Vec::new(),
            review_body: None,
            review_event: ReviewEvent::Comment,
            file_checklist: IndexMap::new(),
            mode,
            started_at: chrono::Utc::now(),
        }
    }

    pub fn approved_comments(&self) -> Vec<&GeneratedComment> {
        self.comments
            .iter()
            .filter(|c| c.status == CommentStatus::Approved)
            .collect()
    }

    pub fn suggested_event(&self) -> ReviewEvent {
        let approved = self.approved_comments();
        let max_severity = approved.iter().map(|c| &c.severity).max();
        match max_severity {
            Some(Severity::Critical) | Some(Severity::Warning) => ReviewEvent::RequestChanges,
            Some(Severity::Suggestion) => ReviewEvent::Comment,
            Some(Severity::Praise) | None => ReviewEvent::Approve,
        }
    }

    pub fn add_comment(&mut self, comment: GeneratedComment) {
        self.comments.push(comment);
    }

    pub fn approve_all(&mut self) {
        for comment in &mut self.comments {
            if comment.status == CommentStatus::Pending {
                comment.status = CommentStatus::Approved;
            }
        }
    }

    /// Auto-generate a markdown summary body from approved comments.
    pub fn generate_body(&self) -> String {
        let approved = self.approved_comments();
        if approved.is_empty() {
            return "Review completed. No inline comments selected.".to_string();
        }

        let mut body = format!(
            "## Review Summary\n\n{} comment(s) selected for this review.\n\n",
            approved.len()
        );

        for sev in &[Severity::Critical, Severity::Warning, Severity::Suggestion, Severity::Praise] {
            let group: Vec<_> = approved.iter().filter(|c| &c.severity == sev).collect();
            if group.is_empty() {
                continue;
            }
            let header = match sev {
                Severity::Critical => "### Critical",
                Severity::Warning => "### Warnings",
                Severity::Suggestion => "### Suggestions",
                Severity::Praise => "### Praise",
            };
            body.push_str(&format!("{}\n\n", header));
            for c in group {
                let location = match (&c.file_path, c.line) {
                    (Some(f), Some(l)) => format!("`{}:{}` — ", f, l),
                    (Some(f), None) => format!("`{}` — ", f),
                    _ => String::new(),
                };
                body.push_str(&format!("- {}{}\n", location, c.effective_body()));
            }
            body.push('\n');
        }

        body
    }

    pub fn generate_body_with_format(&self, fmt: &crate::config::ReviewFormatConfig) -> String {
        let approved = self.approved_comments();
        if approved.is_empty() {
            return String::new();
        }

        let critical_count = approved.iter().filter(|c| c.severity == Severity::Critical).count();
        let warning_count = approved.iter().filter(|c| c.severity == Severity::Warning).count();
        let suggestion_count = approved.iter().filter(|c| c.severity == Severity::Suggestion).count();
        let praise_count = approved.iter().filter(|c| c.severity == Severity::Praise).count();

        let comments_list: String = approved.iter().map(|c| {
            let file = c.file_path.as_deref().unwrap_or("general");
            let line = c.line.map(|l| l.to_string()).unwrap_or_else(|| "-".to_string());
            let severity = c.severity.to_string();
            let body = c.effective_body();
            let source = match &c.source {
                CommentSource::Agent { agent_name, .. } => agent_name.as_str(),
                CommentSource::Manual => "manual",
            };
            fmt.comment_template
                .replace("{file}", file)
                .replace("{line}", &line)
                .replace("{severity}", &severity)
                .replace("{body}", body)
                .replace("{source}", source)
        }).collect::<Vec<_>>().join("\n");

        fmt.body_template
            .replace("{pr_number}", &self.pr_number.to_string())
            .replace("{pr_title}", "") // not available in draft
            .replace("{comment_count}", &approved.len().to_string())
            .replace("{critical_count}", &critical_count.to_string())
            .replace("{warning_count}", &warning_count.to_string())
            .replace("{suggestion_count}", &suggestion_count.to_string())
            .replace("{praise_count}", &praise_count.to_string())
            .replace("{comments_list}", &comments_list)
    }

    pub fn pending_count(&self) -> usize {
        self.comments
            .iter()
            .filter(|c| c.status == CommentStatus::Pending)
            .count()
    }

    pub fn approved_count(&self) -> usize {
        self.approved_comments().len()
    }
}
