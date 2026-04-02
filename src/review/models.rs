use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct GeneratedComment {
    pub id: uuid::Uuid,
    /// GitHub comment id — set when imported from GitHub, used for update/delete.
    pub github_id: Option<u64>,
    /// For reply comments: the github_id of the parent comment in the thread.
    pub parent_github_id: Option<u64>,
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
            github_id: None,
            parent_github_id: None,
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
    /// A top-level review summary fetched from GitHub (not an inline comment).
    GithubReview {
        review_id: u64,
        state: String,
        user: String,
    },
}

impl CommentSource {
    pub fn score(&self) -> u8 {
        match self {
            CommentSource::Manual | CommentSource::GithubReview { .. } => 2,
            CommentSource::Agent { .. } => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    Praise,
    Suggestion,
    Warning,
    Critical,
}

impl Severity {
    pub fn score(&self) -> u8 {
        match self {
            Severity::Praise => 1,
            Severity::Suggestion => 2,
            Severity::Warning => 3,
            Severity::Critical => 4,
        }
    }
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

    /// All comments that will be submitted: Approved + Pending (not explicitly rejected).
    pub fn submittable_comments(&self) -> Vec<&GeneratedComment> {
        self.comments
            .iter()
            .filter(|c| c.status != CommentStatus::Rejected)
            .collect()
    }

    pub fn approved_count(&self) -> usize {
        self.comments.iter().filter(|c| c.status == CommentStatus::Approved).count()
    }

    pub fn pending_count(&self) -> usize {
        self.comments.iter().filter(|c| c.status == CommentStatus::Pending).count()
    }

    pub fn rejected_count(&self) -> usize {
        self.comments.iter().filter(|c| c.status == CommentStatus::Rejected).count()
    }

    pub fn submittable_count(&self) -> usize {
        self.comments.iter().filter(|c| c.status != CommentStatus::Rejected).count()
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

    pub fn add_comment(&mut self, new_comment: GeneratedComment) {
        // If it's a general comment (no file/line), just add it.
        if new_comment.file_path.is_none() || new_comment.line.is_none() {
            self.comments.push(new_comment);
            return;
        }

        let mut should_add = true;
        let mut to_replace = None;

        for (idx, existing) in self.comments.iter().enumerate() {
            // Only compare if they are on the same file and line
            if existing.file_path == new_comment.file_path && existing.line == new_comment.line {
                // Calculate word-level similarity (Jaccard-like overlap)
                let existing_words: std::collections::HashSet<_> = existing.effective_body().split_whitespace().map(|s| s.to_lowercase()).collect();
                let new_words: std::collections::HashSet<_> = new_comment.effective_body().split_whitespace().map(|s| s.to_lowercase()).collect();
                
                if !existing_words.is_empty() && !new_words.is_empty() {
                    let intersection = existing_words.intersection(&new_words).count();
                    let union = existing_words.union(&new_words).count();
                    let similarity = (intersection as f32) / (union as f32);

                    // If they are more than 50% similar, they are likely duplicates/redundant
                    if similarity > 0.5 {
                        // Prioritize: 1. Manual over Agent, 2. Higher Severity, 3. Longer body
                        let existing_priority = (existing.source.score(), existing.severity.score(), existing.body.len());
                        let new_priority = (new_comment.source.score(), new_comment.severity.score(), new_comment.body.len());

                        if new_priority > existing_priority {
                            // The new one is "better", we will replace the existing one
                            to_replace = Some(idx);
                        } else {
                            // The existing one is better or equal, skip adding the new one
                            should_add = false;
                        }
                        break;
                    }
                }
            }
        }

        if let Some(idx) = to_replace {
            self.comments[idx] = new_comment;
        } else if should_add {
            self.comments.push(new_comment);
        }
    }

    pub fn merge_github_reviews(&mut self, reviews: Vec<crate::github::models::GhReview>, comments: Vec<crate::github::models::GhPrComment>) {
        use crate::github::models::GhReviewState;

        // Top-level review summaries (the overall review body)
        for review in reviews {
            if review.body.trim().is_empty() { continue; }
            let state_str = match review.state {
                GhReviewState::Approved => "approved",
                GhReviewState::ChangesRequested => "changes_requested",
                GhReviewState::Commented => "commented",
                GhReviewState::Dismissed => "dismissed",
                GhReviewState::Unknown => "unknown",
            };
            let severity = match review.state {
                GhReviewState::Approved => Severity::Praise,
                GhReviewState::ChangesRequested => Severity::Warning,
                _ => Severity::Suggestion,
            };
            let gen_comment = GeneratedComment {
                id: uuid::Uuid::new_v4(),
                github_id: Some(review.id),
                parent_github_id: None,
                source: CommentSource::GithubReview {
                    review_id: review.id,
                    state: state_str.to_string(),
                    user: review.user.login.clone(),
                },
                file_path: None,
                line: None,
                body: review.body,
                edited_body: None,
                severity,
                status: CommentStatus::Approved,
                created_at: review.submitted_at.unwrap_or_else(chrono::Utc::now),
            };
            self.add_comment(gen_comment);
        }

        // Inline review comments (including threaded replies)
        for gh_c in comments {
            let gen_comment = GeneratedComment {
                id: uuid::Uuid::new_v4(),
                github_id: Some(gh_c.id),
                parent_github_id: gh_c.in_reply_to_id,
                source: CommentSource::Manual,
                file_path: Some(gh_c.path),
                line: gh_c.line,
                body: gh_c.body,
                edited_body: None,
                severity: Severity::Suggestion,
                status: CommentStatus::Approved,
                created_at: gh_c.created_at,
            };
            self.add_comment(gen_comment);
        }
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
                CommentSource::GithubReview { .. } => "github",
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

}
