//! Persistent storage for ReviewDraft — saves the full review state to disk.
//!
//! Location: `~/.local/share/prism/drafts/{owner}-{repo}/pr-{number}.json`
//!
//! The draft is saved after every meaningful change (comments added/removed,
//! file checklist updated, agents complete) and loaded when a PR is opened.
//! This allows reviews to survive app restarts without re-running agents.

use std::path::PathBuf;

use tracing::{debug, info, warn};

use crate::review::models::ReviewDraft;

/// Save a draft to disk.
pub fn save(draft: &ReviewDraft, repo: &str) {
    let path = draft_path(draft.pr_number, repo);
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            warn!("Failed to create drafts dir {}: {e}", parent.display());
            return;
        }
    }
    match serde_json::to_string_pretty(draft) {
        Ok(json) => match std::fs::write(&path, json) {
            Ok(()) => info!(pr = draft.pr_number, repo, "Draft saved"),
            Err(e) => warn!("Failed to write draft to {}: {e}", path.display()),
        },
        Err(e) => warn!("Failed to serialise draft: {e}"),
    }
}

/// Load a previously saved draft. Returns `None` if none exists or on error.
pub fn load(pr_number: u64, repo: &str) -> Option<ReviewDraft> {
    let path = draft_path(pr_number, repo);
    let content = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<ReviewDraft>(&content) {
        Ok(draft) => {
            debug!(pr_number, repo, comments = draft.comments.len(), "Draft loaded from disk");
            Some(draft)
        }
        Err(e) => {
            warn!("Failed to parse draft at {}: {e}", path.display());
            None
        }
    }
}

/// Delete the draft file for a PR (e.g. after publishing).
pub fn delete(pr_number: u64, repo: &str) {
    let path = draft_path(pr_number, repo);
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            warn!("Failed to delete draft {}: {e}", path.display());
        }
    }
}

/// Remove draft files for PRs that are no longer open (merged/closed).
pub fn prune_closed(repo: &str, open_numbers: &[u64]) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let repo_slug = repo.replace('/', "-");
    let dir = PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("prism")
        .join("drafts")
        .join(repo_slug);

    let open_set: std::collections::HashSet<u64> = open_numbers.iter().copied().collect();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(num_str) = name.strip_prefix("pr-").and_then(|s| s.strip_suffix(".json")) {
            if let Ok(n) = num_str.parse::<u64>() {
                if !open_set.contains(&n) {
                    if let Err(e) = std::fs::remove_file(entry.path()) {
                        warn!("Failed to prune draft for PR {n}: {e}");
                    } else {
                        info!(pr = n, repo, "Pruned draft for closed/merged PR");
                    }
                }
            }
        }
    }
}

fn draft_path(pr_number: u64, repo: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let repo_slug = repo.replace('/', "-");
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("prism")
        .join("drafts")
        .join(repo_slug)
        .join(format!("pr-{pr_number}.json"))
}
