use anyhow::Result;
use tracing::{info, warn};

use crate::github::api::GitHubApi;
use crate::github::models::ReviewRequest;
use crate::review::models::{CommentStatus, GeneratedComment, ReviewDraft};

pub struct ReviewPublisher {
    api: GitHubApi,
}

impl ReviewPublisher {
    pub fn new(api: GitHubApi) -> Self {
        Self { api }
    }

    /// Publish all non-rejected comments (Approved + Pending) as a single PR review.
    pub async fn publish(&self, draft: &ReviewDraft) -> Result<()> {
        let approved: Vec<&GeneratedComment> = draft.submittable_comments();

        if approved.is_empty() {
            warn!("No submittable comments — submitting empty review body only");
        }

        // Skip comments that already exist on GitHub (github_id is set) — re-posting
        // them would create duplicates. Only new, locally-generated comments are submitted.
        let inline_comments: Vec<crate::github::models::ReviewComment> = approved
            .iter()
            .filter(|c| c.github_id.is_none())
            .filter_map(|c| {
                let path = c.file_path.as_ref()?;
                let line = c.line?;
                Some(crate::github::models::ReviewComment {
                    path: path.clone(),
                    line,
                    body: c.effective_body().to_string(),
                })
            })
            .collect();

        let review_request = ReviewRequest {
            body: draft.review_body.clone().unwrap_or_default(),
            event: draft.review_event.as_github_str().to_string(),
            comments: inline_comments,
        };

        info!(
            pr_number = draft.pr_number,
            comment_count = approved.len(),
            event = draft.review_event.as_github_str(),
            "Publishing review"
        );

        self.api
            .submit_review(draft.pr_number, review_request)
            .await?;

        info!("Review published successfully");
        Ok(())
    }

    /// Publish only comments that have been explicitly approved.
    pub async fn publish_selected(
        &self,
        draft: &ReviewDraft,
        selected_ids: &[uuid::Uuid],
    ) -> Result<()> {
        let selected: Vec<&GeneratedComment> = draft
            .comments
            .iter()
            .filter(|c| selected_ids.contains(&c.id) && c.status == CommentStatus::Approved)
            .collect();

        let inline_comments: Vec<crate::github::models::ReviewComment> = selected
            .iter()
            .filter_map(|c| {
                let path = c.file_path.as_ref()?;
                let line = c.line?;
                Some(crate::github::models::ReviewComment {
                    path: path.clone(),
                    line,
                    body: c.effective_body().to_string(),
                })
            })
            .collect();

        let review_request = ReviewRequest {
            body: draft.review_body.clone().unwrap_or_default(),
            event: draft.review_event.as_github_str().to_string(),
            comments: inline_comments,
        };

        self.api
            .submit_review(draft.pr_number, review_request)
            .await?;

        Ok(())
    }
}
