use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Lightweight PR summary for list views.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrSummary {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub base_branch: String,
    pub head_branch: String,
    pub state: PrState,
    pub draft: bool,
    pub additions: u32,
    pub deletions: u32,
    pub changed_files: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub html_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

impl std::fmt::Display for PrState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrState::Open => write!(f, "open"),
            PrState::Closed => write!(f, "closed"),
            PrState::Merged => write!(f, "merged"),
        }
    }
}

/// Full PR details needed for review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrDetails {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub author: String,
    pub base_branch: String,
    pub head_branch: String,
    pub state: PrState,
    pub draft: bool,
    pub html_url: String,
    pub additions: u32,
    pub deletions: u32,
    pub changed_files: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub labels: Vec<String>,
    pub reviewers: Vec<String>,
    pub repo_language: Option<String>,
}

/// A single inline review comment for GitHub's review API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewComment {
    pub path: String,
    pub line: u32,
    pub body: String,
}

/// Request body for submitting a PR review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRequest {
    pub body: String,
    pub event: String,
    pub comments: Vec<ReviewComment>,
}

/// Raw GitHub API PR object (subset).
#[derive(Debug, Deserialize)]
pub(crate) struct GhPr {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub user: GhUser,
    pub base: GhRef,
    pub head: GhRef,
    pub state: String,
    pub draft: Option<bool>,
    pub html_url: String,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub changed_files: Option<u32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub labels: Option<Vec<GhLabel>>,
    pub requested_reviewers: Option<Vec<GhUser>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GhUser {
    pub login: String,
}

/// State of an existing GitHub review.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GhReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    #[serde(other)]
    Unknown,
}

/// A review fetched from GitHub (existing review on PR).
#[derive(Debug, Clone, Deserialize)]
pub struct GhReview {
    pub id: u64,
    pub user: GhUser,
    pub body: String,
    pub state: GhReviewState,
    pub submitted_at: Option<DateTime<Utc>>,
}

/// Typed response for the /user endpoint.
#[derive(Debug, Deserialize)]
pub(crate) struct CurrentUser {
    pub login: String,
}

/// An existing inline PR review comment from GitHub.
#[derive(Debug, Clone, Deserialize)]
pub struct GhPrComment {
    pub id: u64,
    pub user: GhUser,
    pub body: String,
    pub path: String,
    pub line: Option<u32>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GhRef {
    #[serde(rename = "ref")]
    pub ref_name: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GhLabel {
    pub name: String,
}

impl From<GhPr> for PrSummary {
    fn from(pr: GhPr) -> Self {
        let state = match pr.state.as_str() {
            "open" => PrState::Open,
            "closed" => PrState::Closed,
            _ => PrState::Closed,
        };
        PrSummary {
            number: pr.number,
            title: pr.title,
            author: pr.user.login,
            base_branch: pr.base.ref_name,
            head_branch: pr.head.ref_name,
            state,
            draft: pr.draft.unwrap_or(false),
            additions: pr.additions.unwrap_or(0),
            deletions: pr.deletions.unwrap_or(0),
            changed_files: pr.changed_files.unwrap_or(0),
            created_at: pr.created_at,
            updated_at: pr.updated_at,
            html_url: pr.html_url,
        }
    }
}
