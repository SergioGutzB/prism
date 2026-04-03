use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CommentStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ReviewMode {
    AiOnly,
    ManualOnly,
    Hybrid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        // GitHub-sourced comments: deduplicate by github_id (exact match, never duplicate).
        if let Some(new_gh_id) = new_comment.github_id {
            if self.comments.iter().any(|c| c.github_id == Some(new_gh_id)) {
                return; // already present
            }
            self.comments.push(new_comment);
            return;
        }

        // Agent/manual comments without a file+line: just push (no meaningful dedup key).
        if new_comment.file_path.is_none() || new_comment.line.is_none() {
            self.comments.push(new_comment);
            return;
        }

        // Agent comments at a specific file+line: fuzzy-dedup by body similarity.
        let mut should_add = true;
        let mut to_replace = None;

        for (idx, existing) in self.comments.iter().enumerate() {
            if existing.github_id.is_some() { continue; } // don't replace GitHub comments
            if existing.file_path == new_comment.file_path && existing.line == new_comment.line {
                let existing_words: std::collections::HashSet<_> = existing.effective_body().split_whitespace().map(|s| s.to_lowercase()).collect();
                let new_words: std::collections::HashSet<_> = new_comment.effective_body().split_whitespace().map(|s| s.to_lowercase()).collect();

                if !existing_words.is_empty() && !new_words.is_empty() {
                    let intersection = existing_words.intersection(&new_words).count();
                    let union = existing_words.union(&new_words).count();
                    let similarity = (intersection as f32) / (union as f32);

                    if similarity > 0.5 {
                        let existing_priority = (existing.source.score(), existing.severity.score(), existing.body.len());
                        let new_priority = (new_comment.source.score(), new_comment.severity.score(), new_comment.body.len());

                        if new_priority > existing_priority {
                            to_replace = Some(idx);
                        } else {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_comment(file: &str, line: u32, body: &str, severity: Severity) -> GeneratedComment {
        GeneratedComment::new(
            CommentSource::Agent {
                agent_id: "test".into(),
                agent_name: "Test".into(),
                agent_icon: "🤖".into(),
            },
            body.to_string(),
            severity,
            Some(file.to_string()),
            Some(line),
        )
    }

    fn manual_comment(file: &str, line: u32, body: &str, severity: Severity) -> GeneratedComment {
        GeneratedComment::new(
            CommentSource::Manual,
            body.to_string(),
            severity,
            Some(file.to_string()),
            Some(line),
        )
    }

    fn gh_comment(github_id: u64, body: &str) -> GeneratedComment {
        let mut c = GeneratedComment::new(
            CommentSource::GithubReview { review_id: github_id, state: "commented".into(), user: "user".into() },
            body.to_string(),
            Severity::Suggestion,
            None,
            None,
        );
        c.github_id = Some(github_id);
        c
    }

    // ── add_comment: github_id dedup ──────────────────────────────────────────

    #[test]
    fn add_comment_same_github_id_not_duplicated() {
        let mut draft = ReviewDraft::new(1, ReviewMode::ManualOnly);
        draft.add_comment(gh_comment(42, "first body"));
        draft.add_comment(gh_comment(42, "second body")); // same github_id → rejected
        assert_eq!(draft.comments.len(), 1);
        assert_eq!(draft.comments[0].body, "first body");
    }

    #[test]
    fn add_comment_different_github_ids_both_kept() {
        let mut draft = ReviewDraft::new(1, ReviewMode::ManualOnly);
        draft.add_comment(gh_comment(1, "body a"));
        draft.add_comment(gh_comment(2, "body b"));
        assert_eq!(draft.comments.len(), 2);
    }

    // ── add_comment: no file/line path ────────────────────────────────────────

    #[test]
    fn add_comment_no_file_path_always_appended() {
        let mut draft = ReviewDraft::new(1, ReviewMode::AiOnly);
        let make = |body: &str| GeneratedComment::new(
            CommentSource::Manual, body.to_string(), Severity::Suggestion, None, None,
        );
        draft.add_comment(make("summary 1"));
        draft.add_comment(make("summary 2"));
        // Summaries have no file path — always pushed, no dedup
        assert_eq!(draft.comments.len(), 2);
    }

    // ── add_comment: fuzzy dedup by Jaccard similarity ────────────────────────

    #[test]
    fn add_comment_fuzzy_dedup_high_similarity_keeps_one() {
        let mut draft = ReviewDraft::new(1, ReviewMode::AiOnly);
        // >50% word overlap → second is a near-duplicate of the first.
        // intersection={this,function,is,too,long,and,be,refactored} = 8; union = 13 → 0.615 > 0.5
        let c1 = agent_comment("src/lib.rs", 10, "This function is too long and needs to be refactored now", Severity::Warning);
        let c2 = agent_comment("src/lib.rs", 10, "This function is too long and should be refactored soon", Severity::Warning);
        draft.add_comment(c1);
        draft.add_comment(c2);
        assert_eq!(draft.comments.len(), 1);
    }

    #[test]
    fn add_comment_fuzzy_dedup_different_lines_both_kept() {
        let mut draft = ReviewDraft::new(1, ReviewMode::AiOnly);
        // Same text but different lines → both kept
        let c1 = agent_comment("src/lib.rs", 10, "missing error handling", Severity::Warning);
        let c2 = agent_comment("src/lib.rs", 20, "missing error handling", Severity::Warning);
        draft.add_comment(c1);
        draft.add_comment(c2);
        assert_eq!(draft.comments.len(), 2);
    }

    #[test]
    fn add_comment_fuzzy_dedup_different_files_both_kept() {
        let mut draft = ReviewDraft::new(1, ReviewMode::AiOnly);
        let c1 = agent_comment("src/a.rs", 5, "missing error handling", Severity::Warning);
        let c2 = agent_comment("src/b.rs", 5, "missing error handling", Severity::Warning);
        draft.add_comment(c1);
        draft.add_comment(c2);
        assert_eq!(draft.comments.len(), 2);
    }

    #[test]
    fn add_comment_fuzzy_dedup_low_similarity_both_kept() {
        let mut draft = ReviewDraft::new(1, ReviewMode::AiOnly);
        // Completely different bodies → both kept
        let c1 = agent_comment("src/lib.rs", 5, "missing error handling here", Severity::Warning);
        let c2 = agent_comment("src/lib.rs", 5, "consider using a trait object instead", Severity::Suggestion);
        draft.add_comment(c1);
        draft.add_comment(c2);
        assert_eq!(draft.comments.len(), 2);
    }

    #[test]
    fn add_comment_manual_replaces_agent_at_same_location() {
        let mut draft = ReviewDraft::new(1, ReviewMode::AiOnly);
        // Agent comment added first with identical-enough body
        let agent = agent_comment("src/lib.rs", 10, "this function is very long please refactor it", Severity::Warning);
        let manual = manual_comment("src/lib.rs", 10, "this function is very long you should refactor it now", Severity::Warning);
        draft.add_comment(agent);
        draft.add_comment(manual); // Manual has higher source score → should replace agent
        assert_eq!(draft.comments.len(), 1);
        assert!(matches!(draft.comments[0].source, CommentSource::Manual));
    }

    #[test]
    fn add_comment_higher_severity_replaces_lower_at_same_location() {
        let mut draft = ReviewDraft::new(1, ReviewMode::AiOnly);
        let low = agent_comment("src/lib.rs", 1, "this function is too complex and long", Severity::Suggestion);
        let high = agent_comment("src/lib.rs", 1, "this function is too complex and buggy", Severity::Critical);
        draft.add_comment(low);
        draft.add_comment(high);
        assert_eq!(draft.comments.len(), 1);
        assert_eq!(draft.comments[0].severity, Severity::Critical);
    }

    // ── Severity ordering and parsing ─────────────────────────────────────────

    #[test]
    fn severity_score_ordering() {
        assert!(Severity::Critical.score() > Severity::Warning.score());
        assert!(Severity::Warning.score() > Severity::Suggestion.score());
        assert!(Severity::Suggestion.score() > Severity::Praise.score());
    }

    #[test]
    fn severity_from_str_roundtrip() {
        for (s, expected) in [
            ("critical", Severity::Critical),
            ("warning", Severity::Warning),
            ("suggestion", Severity::Suggestion),
            ("praise", Severity::Praise),
        ] {
            let parsed: Severity = s.parse().expect("should parse");
            assert_eq!(parsed, expected);
        }
    }

    #[test]
    fn severity_from_str_unknown_errors() {
        let result: Result<Severity, _> = "blocker".parse();
        assert!(result.is_err());
    }

    // ── CommentSource score ───────────────────────────────────────────────────

    #[test]
    fn comment_source_scores() {
        assert_eq!(CommentSource::Manual.score(), 2);
        assert_eq!(CommentSource::GithubReview { review_id: 1, state: "".into(), user: "".into() }.score(), 2);
        assert_eq!(CommentSource::Agent { agent_id: "a".into(), agent_name: "A".into(), agent_icon: "".into() }.score(), 1);
    }

    // ── suggested_event ───────────────────────────────────────────────────────

    #[test]
    fn suggested_event_no_approved_comments_returns_approve() {
        let draft = ReviewDraft::new(1, ReviewMode::ManualOnly);
        assert_eq!(draft.suggested_event(), ReviewEvent::Approve);
    }

    #[test]
    fn suggested_event_critical_comment_returns_request_changes() {
        let mut draft = ReviewDraft::new(1, ReviewMode::ManualOnly);
        let mut c = agent_comment("a.rs", 1, "critical issue", Severity::Critical);
        c.status = CommentStatus::Approved;
        draft.comments.push(c);
        assert_eq!(draft.suggested_event(), ReviewEvent::RequestChanges);
    }

    #[test]
    fn suggested_event_warning_returns_request_changes() {
        let mut draft = ReviewDraft::new(1, ReviewMode::ManualOnly);
        let mut c = agent_comment("a.rs", 1, "warning issue", Severity::Warning);
        c.status = CommentStatus::Approved;
        draft.comments.push(c);
        assert_eq!(draft.suggested_event(), ReviewEvent::RequestChanges);
    }

    #[test]
    fn suggested_event_suggestion_returns_comment() {
        let mut draft = ReviewDraft::new(1, ReviewMode::ManualOnly);
        let mut c = agent_comment("a.rs", 1, "a suggestion", Severity::Suggestion);
        c.status = CommentStatus::Approved;
        draft.comments.push(c);
        assert_eq!(draft.suggested_event(), ReviewEvent::Comment);
    }

    #[test]
    fn suggested_event_only_rejected_comments_returns_approve() {
        let mut draft = ReviewDraft::new(1, ReviewMode::ManualOnly);
        let mut c = agent_comment("a.rs", 1, "rejected warning", Severity::Warning);
        c.status = CommentStatus::Rejected;
        draft.comments.push(c);
        // Rejected comments are not in approved_comments() → no max severity → Approve
        assert_eq!(draft.suggested_event(), ReviewEvent::Approve);
    }

    // ── count helpers ─────────────────────────────────────────────────────────

    #[test]
    fn draft_counts_are_consistent() {
        let mut draft = ReviewDraft::new(1, ReviewMode::AiOnly);
        let c1 = agent_comment("a.rs", 1, "pending", Severity::Suggestion);
        let mut c2 = agent_comment("b.rs", 2, "approved", Severity::Warning);
        let mut c3 = agent_comment("c.rs", 3, "rejected", Severity::Critical);
        c2.status = CommentStatus::Approved;
        c3.status = CommentStatus::Rejected;
        draft.comments.extend([c1, c2, c3]);

        assert_eq!(draft.pending_count(), 1);
        assert_eq!(draft.approved_count(), 1);
        assert_eq!(draft.rejected_count(), 1);
        assert_eq!(draft.submittable_count(), 2); // pending + approved
    }
}
