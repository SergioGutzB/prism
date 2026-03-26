use anyhow::{Context, Result};
use serde_json::Value;
use tracing::debug;

use crate::github::client::GitHubClient;
use crate::github::models::{GhPr, PrDetails, PrState, PrSummary, ReviewRequest};

pub struct GitHubApi {
    client: GitHubClient,
}

impl GitHubApi {
    pub fn new(client: GitHubClient) -> Self {
        Self { client }
    }

    /// List open pull requests for the configured repository.
    pub async fn list_prs(&self, per_page: u32) -> Result<Vec<PrSummary>> {
        let url = format!(
            "{}/repos/{}/{}/pulls?state=open&per_page={}&sort=updated&direction=desc",
            self.client.base_url, self.client.owner, self.client.repo, per_page
        );

        debug!("GET {}", url);

        let response = self
            .client
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch PR list")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        let prs: Vec<GhPr> = response.json().await.context("Failed to parse PR list")?;

        Ok(prs.into_iter().map(PrSummary::from).collect())
    }

    /// Get the unified diff for a specific PR.
    pub async fn get_pr_diff(&self, pr_number: u64) -> Result<String> {
        let url = format!(
            "{}/repos/{}/{}/pulls/{}",
            self.client.base_url, self.client.owner, self.client.repo, pr_number
        );

        debug!("GET diff for PR #{}", pr_number);

        let response = self
            .client
            .client
            .get(&url)
            .header("Accept", "application/vnd.github.diff")
            .send()
            .await
            .context("Failed to fetch PR diff")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        response.text().await.context("Failed to read diff body")
    }

    /// Get full PR details.
    pub async fn get_pr_details(&self, pr_number: u64) -> Result<PrDetails> {
        let url = format!(
            "{}/repos/{}/{}/pulls/{}",
            self.client.base_url, self.client.owner, self.client.repo, pr_number
        );

        debug!("GET PR #{} details", pr_number);

        let response = self
            .client
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch PR details")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        let pr: GhPr = response.json().await.context("Failed to parse PR details")?;

        // Fetch repo language separately
        let repo_language = self.get_repo_language().await.ok().flatten();

        let state = match pr.state.as_str() {
            "open" => PrState::Open,
            "closed" => PrState::Closed,
            _ => PrState::Closed,
        };

        let labels = pr
            .labels
            .unwrap_or_default()
            .into_iter()
            .map(|l| l.name)
            .collect();

        let reviewers = pr
            .requested_reviewers
            .unwrap_or_default()
            .into_iter()
            .map(|u| u.login)
            .collect();

        Ok(PrDetails {
            number: pr.number,
            title: pr.title,
            body: pr.body.unwrap_or_default(),
            author: pr.user.login,
            base_branch: pr.base.ref_name,
            head_branch: pr.head.ref_name,
            state,
            draft: pr.draft.unwrap_or(false),
            html_url: pr.html_url,
            additions: pr.additions.unwrap_or(0),
            deletions: pr.deletions.unwrap_or(0),
            changed_files: pr.changed_files.unwrap_or(0),
            created_at: pr.created_at,
            updated_at: pr.updated_at,
            labels,
            reviewers,
            repo_language,
        })
    }

    /// Submit a PR review.
    pub async fn submit_review(&self, pr_number: u64, review: ReviewRequest) -> Result<()> {
        let url = format!(
            "{}/repos/{}/{}/pulls/{}/reviews",
            self.client.base_url, self.client.owner, self.client.repo, pr_number
        );

        debug!("POST review to PR #{}", pr_number);

        let response = self
            .client
            .client
            .post(&url)
            .json(&review)
            .send()
            .await
            .context("Failed to submit review")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        Ok(())
    }

    /// Get the primary language of the repository.
    async fn get_repo_language(&self) -> Result<Option<String>> {
        let url = format!(
            "{}/repos/{}/{}",
            self.client.base_url, self.client.owner, self.client.repo
        );

        let response = self.client.client.get(&url).send().await?;
        if !response.status().is_success() {
            return Ok(None);
        }

        let data: Value = response.json().await?;
        Ok(data["language"].as_str().map(|s| s.to_string()))
    }
}
