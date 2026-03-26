use std::sync::Arc;
use crate::github::models::PrDetails;
use crate::tickets::models::Ticket;

#[derive(Debug, Clone)]
pub struct ReviewContext {
    pub pr_number: u64,
    pub pr_title: String,
    pub pr_description: String,
    pub pr_author: String,
    pub base_branch: String,
    pub head_branch: String,
    pub pr_url: String,
    pub raw_diff: Arc<str>,
    pub changed_files: Vec<ChangedFile>,
    pub diff_stats: DiffStats,
    pub ticket: Option<Ticket>,
    pub repo_language: Option<String>,
}

impl ReviewContext {
    /// Build a `ReviewContext` from GitHub PR details + raw diff text.
    pub fn from_pr(pr: &PrDetails, diff: &str, ticket: Option<Ticket>) -> Self {
        let raw_diff: Arc<str> = Arc::from(diff);
        let diff_stats = DiffStats::from_diff(diff);
        let changed_files = parse_changed_files(diff);

        Self {
            pr_number: pr.number,
            pr_title: pr.title.clone(),
            pr_description: pr.body.clone(),
            pr_author: pr.author.clone(),
            base_branch: pr.base_branch.clone(),
            head_branch: pr.head_branch.clone(),
            pr_url: pr.html_url.clone(),
            raw_diff,
            changed_files,
            diff_stats,
            ticket,
            repo_language: pr.repo_language.clone(),
        }
    }

    /// Build a human-readable file list string.
    pub fn file_list_text(&self) -> String {
        self.changed_files
            .iter()
            .map(|f| {
                let status_str = match &f.status {
                    FileStatus::Added => "A".to_string(),
                    FileStatus::Modified => "M".to_string(),
                    FileStatus::Deleted => "D".to_string(),
                    FileStatus::Renamed { from } => format!("R:{}", from),
                };
                format!(
                    "[{}] {} (+{} -{})",
                    status_str, f.path, f.additions, f.deletions
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Truncate the diff to approximately `max_tokens` tokens (rough estimate: 4 chars/token).
    pub fn truncated_diff(&self, max_tokens: u32) -> &str {
        let max_chars = (max_tokens * 4) as usize;
        let diff = &*self.raw_diff;
        if diff.len() <= max_chars {
            diff
        } else {
            &diff[..max_chars]
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChangedFile {
    pub path: String,
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
    pub diff_hunk: String,
    pub is_generated: bool,
}

#[derive(Debug, Clone)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed { from: String },
}

#[derive(Debug, Clone, Default)]
pub struct DiffStats {
    pub total_additions: u32,
    pub total_deletions: u32,
    pub files_changed: u32,
    pub estimated_tokens: u32,
}

/// Parse a unified diff and extract per-file metadata.
fn parse_changed_files(diff: &str) -> Vec<ChangedFile> {
    let mut files: Vec<ChangedFile> = Vec::new();
    let mut current_path: Option<String> = None;
    let mut additions: u32 = 0;
    let mut deletions: u32 = 0;
    let mut hunk_buf = String::new();

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            // Save the previous file
            if let Some(path) = current_path.take() {
                files.push(ChangedFile {
                    path,
                    status: FileStatus::Modified,
                    additions,
                    deletions,
                    diff_hunk: std::mem::take(&mut hunk_buf),
                    is_generated: false,
                });
            }
            additions = 0;
            deletions = 0;
            // Extract path: "diff --git a/foo.rs b/foo.rs" → "foo.rs"
            if let Some(b_part) = line.split(" b/").nth(1) {
                current_path = Some(b_part.to_string());
            }
        } else if line.starts_with('+') && !line.starts_with("+++") {
            additions += 1;
            hunk_buf.push_str(line);
            hunk_buf.push('\n');
        } else if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
            hunk_buf.push_str(line);
            hunk_buf.push('\n');
        } else {
            hunk_buf.push_str(line);
            hunk_buf.push('\n');
        }
    }

    // Save last file
    if let Some(path) = current_path.take() {
        let is_generated = path.ends_with(".lock")
            || path.contains("generated")
            || path.starts_with("dist/")
            || path.starts_with("build/");
        files.push(ChangedFile {
            path,
            status: FileStatus::Modified,
            additions,
            deletions,
            diff_hunk: hunk_buf,
            is_generated,
        });
    }

    files
}

impl DiffStats {
    pub fn from_diff(diff: &str) -> Self {
        let mut additions: u32 = 0;
        let mut deletions: u32 = 0;
        let mut files: u32 = 0;

        for line in diff.lines() {
            if line.starts_with("+++) ") || line.starts_with("--- ") {
                // skip header lines
            } else if line.starts_with("diff --git") {
                files += 1;
            } else if line.starts_with('+') {
                additions += 1;
            } else if line.starts_with('-') {
                deletions += 1;
            }
        }

        let estimated_tokens = (diff.len() / 4) as u32;

        Self {
            total_additions: additions,
            total_deletions: deletions,
            files_changed: files,
            estimated_tokens,
        }
    }
}
