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
///
/// Deduplicates comments by `github_id` on load so that drafts corrupted by an
/// older bug (reviews added multiple times per session) are self-healing.
pub fn load(pr_number: u64, repo: &str) -> Option<ReviewDraft> {
    let path = draft_path(pr_number, repo);
    let content = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<ReviewDraft>(&content) {
        Ok(mut draft) => {
            let before = draft.comments.len();
            dedup_comments(&mut draft.comments);
            let after = draft.comments.len();
            if after < before {
                warn!(pr_number, repo, removed = before - after, "Removed duplicate comments from draft on load");
                // Persist the cleaned-up version immediately so it doesn't accumulate again
                save(&draft, repo);
            }
            debug!(pr_number, repo, comments = draft.comments.len(), "Draft loaded from disk");
            Some(draft)
        }
        Err(e) => {
            warn!("Failed to parse draft at {}: {e}", path.display());
            None
        }
    }
}

/// Remove duplicate comments in-place. Priority order when duplicates exist:
/// keep the entry with the highest `source.score()` (Manual/GitHub > Agent),
/// then highest `severity.score()`, then longest body.
fn dedup_comments(comments: &mut Vec<crate::review::models::GeneratedComment>) {
    use std::collections::HashMap;

    // Deduplicate by github_id (keeping the "best" entry)
    let mut by_gh_id: HashMap<u64, usize> = HashMap::new(); // github_id → index to keep
    let mut to_remove: Vec<usize> = Vec::new();

    for (i, c) in comments.iter().enumerate() {
        if let Some(gid) = c.github_id {
            if let Some(&keep_idx) = by_gh_id.get(&gid) {
                // Compare with the entry we already plan to keep
                let keep = &comments[keep_idx];
                let keep_priority = (keep.source.score(), keep.severity.score(), keep.body.len());
                let cur_priority  = (c.source.score(),    c.severity.score(),    c.body.len());
                if cur_priority > keep_priority {
                    to_remove.push(keep_idx);
                    by_gh_id.insert(gid, i);
                } else {
                    to_remove.push(i);
                }
            } else {
                by_gh_id.insert(gid, i);
            }
        }
    }

    // Deduplicate by UUID (should never happen, but guard anyway)
    let mut seen_ids = std::collections::HashSet::new();
    for (i, c) in comments.iter().enumerate() {
        if !seen_ids.insert(c.id) {
            to_remove.push(i);
        }
    }

    if to_remove.is_empty() { return; }

    to_remove.sort_unstable();
    to_remove.dedup();
    // Remove in reverse order to preserve indices
    for &idx in to_remove.iter().rev() {
        comments.remove(idx);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::models::{CommentSource, CommentStatus, GeneratedComment, Severity};

    fn make_gh(github_id: u64, body: &str, severity: Severity, source_score: u8) -> GeneratedComment {
        let source = if source_score >= 2 {
            CommentSource::GithubReview { review_id: github_id, state: "commented".into(), user: "u".into() }
        } else {
            CommentSource::Agent { agent_id: "a".into(), agent_name: "A".into(), agent_icon: "".into() }
        };
        let mut c = GeneratedComment::new(source, body.to_string(), severity, None, None);
        c.github_id = Some(github_id);
        c
    }

    fn make_uuid_dup(original: &GeneratedComment) -> GeneratedComment {
        // Same UUID as the original — simulates the corruption bug
        let mut dup = original.clone();
        dup.body = "duplicate body".to_string();
        dup
    }

    // ── dedup_comments: by github_id ─────────────────────────────────────────

    #[test]
    fn dedup_removes_duplicate_github_id() {
        let c1 = make_gh(42, "body v1", Severity::Suggestion, 2);
        let c2 = make_gh(42, "body v2", Severity::Suggestion, 2); // same github_id
        let mut comments = vec![c1, c2];
        dedup_comments(&mut comments);
        assert_eq!(comments.len(), 1);
    }

    #[test]
    fn dedup_keeps_higher_priority_on_duplicate_github_id() {
        // c1: agent (score=1, suggestion), c2: github (score=2, suggestion) → keep c2
        let c1 = make_gh(99, "agent body", Severity::Suggestion, 1);
        let c2 = make_gh(99, "github body", Severity::Suggestion, 2);
        let mut comments = vec![c1, c2];
        dedup_comments(&mut comments);
        assert_eq!(comments.len(), 1);
        assert!(matches!(comments[0].source, CommentSource::GithubReview { .. }));
    }

    #[test]
    fn dedup_keeps_higher_severity_among_same_source() {
        let c1 = make_gh(7, "body", Severity::Suggestion, 2);
        let c2 = make_gh(7, "body", Severity::Critical, 2); // same source score, higher severity
        let mut comments = vec![c1, c2];
        dedup_comments(&mut comments);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].severity, Severity::Critical);
    }

    #[test]
    fn dedup_no_duplicates_unchanged() {
        let c1 = make_gh(1, "body a", Severity::Warning, 2);
        let c2 = make_gh(2, "body b", Severity::Suggestion, 2);
        let mut comments = vec![c1, c2];
        dedup_comments(&mut comments);
        assert_eq!(comments.len(), 2);
    }

    #[test]
    fn dedup_removes_uuid_duplicate() {
        let original = make_gh(0, "original", Severity::Warning, 1);
        // Remove github_id so dedup-by-github_id doesn't interfere
        let mut base = original.clone();
        base.github_id = None;
        let dup = make_uuid_dup(&base); // same UUID, same id
        let mut comments = vec![base, dup];
        dedup_comments(&mut comments);
        assert_eq!(comments.len(), 1);
    }

    #[test]
    fn dedup_empty_vec_is_safe() {
        let mut comments: Vec<GeneratedComment> = vec![];
        dedup_comments(&mut comments); // must not panic
        assert!(comments.is_empty());
    }

    #[test]
    fn dedup_no_github_id_comments_untouched() {
        // Comments without github_id are not touched by the github_id dedup pass
        let make_agent = |body: &str| GeneratedComment::new(
            CommentSource::Agent { agent_id: "a".into(), agent_name: "A".into(), agent_icon: "".into() },
            body.to_string(),
            Severity::Suggestion,
            Some("src/lib.rs".into()),
            Some(1),
        );
        let c1 = make_agent("agent comment one");
        let c2 = make_agent("agent comment two");
        let mut comments = vec![c1, c2];
        dedup_comments(&mut comments);
        // Both have unique UUIDs and no github_id → both kept
        assert_eq!(comments.len(), 2);
    }

    #[test]
    fn dedup_six_copies_reduced_to_one() {
        // Reproduce the real-world bug: same github_id added 6 times across sessions
        let template = make_gh(55, "review body", Severity::Suggestion, 2);
        let comments_6x: Vec<GeneratedComment> = (0..6).map(|_| template.clone()).collect();
        let mut comments = comments_6x;
        dedup_comments(&mut comments);
        assert_eq!(comments.len(), 1);
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
