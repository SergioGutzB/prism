//! Persistent cache for AI review results, keyed by git blob SHA per file.
//!
//! Each cache entry stores the comments a specific agent produced for a specific
//! file version (identified by its git blob SHA). If the file hasn't changed
//! between PR pushes the cached comments are reused, saving tokens and time.
//!
//! Cache location: `~/.local/share/prism/cache/{owner}-{repo}/pr-{number}.json`

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::review::models::{CommentSource, GeneratedComment, Severity};

// ── Serialisable comment ──────────────────────────────────────────────────────

/// Serialisable snapshot of a `GeneratedComment`, stored in the cache.
/// Round-trips back to a `GeneratedComment` via `to_comment()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedComment {
    pub file_path: Option<String>,
    pub line: Option<u32>,
    pub body: String,
    pub severity: String,
    pub agent_id: String,
    pub agent_name: String,
    pub agent_icon: String,
}

impl CachedComment {
    pub fn from_comment(c: &GeneratedComment) -> Self {
        let (agent_id, agent_name, agent_icon) = match &c.source {
            CommentSource::Agent { agent_id, agent_name, agent_icon } => {
                (agent_id.clone(), agent_name.clone(), agent_icon.clone())
            }
            CommentSource::Manual => (
                "manual".to_string(),
                "Manual".to_string(),
                "✏️".to_string(),
            ),
            CommentSource::GithubReview { user, .. } => (
                "github".to_string(),
                user.clone(),
                "💬".to_string(),
            ),
        };
        Self {
            file_path: c.file_path.clone(),
            line: c.line,
            body: c.effective_body().to_string(),
            severity: c.severity.to_string(),
            agent_id,
            agent_name,
            agent_icon,
        }
    }

    pub fn to_comment(&self) -> GeneratedComment {
        GeneratedComment::new(
            CommentSource::Agent {
                agent_id: self.agent_id.clone(),
                agent_name: self.agent_name.clone(),
                agent_icon: self.agent_icon.clone(),
            },
            self.body.clone(),
            self.severity.parse::<Severity>().unwrap_or(Severity::Suggestion),
            self.file_path.clone(),
            self.line,
        )
    }
}

// ── Per-file, per-agent cache entry ──────────────────────────────────────────

/// Result of one agent running on one file version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileCacheEntry {
    /// Git blob SHA of the file at review time.
    pub blob_sha: String,
    pub reviewed_at: DateTime<Utc>,
    pub comments: Vec<CachedComment>,
}

// ── ReviewCache ───────────────────────────────────────────────────────────────

/// Cached review results for a single PR.
///
/// Indexed as `entries[agent_id][file_path] = FileCacheEntry`.
/// Additionally, general (non-file) comments are stored under the key
/// `__general__` in the inner map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewCache {
    pub schema_version: u32,
    pub pr_number: u64,
    /// `"owner/repo"` string — used for the cache path.
    pub repo: String,
    /// agent_id → file_path → entry
    pub entries: HashMap<String, HashMap<String, FileCacheEntry>>,
}

impl ReviewCache {
    const SCHEMA_VERSION: u32 = 1;
    /// Pseudo-path used for general (non-file-specific) comments.
    pub const GENERAL_KEY: &'static str = "__general__";

    pub fn new(pr_number: u64, repo: &str) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            pr_number,
            repo: repo.to_string(),
            entries: HashMap::new(),
        }
    }

    // ── Persistence ──────────────────────────────────────────────────────────

    pub fn load(pr_number: u64, repo: &str) -> Option<Self> {
        let path = cache_path(pr_number, repo);
        let content = std::fs::read_to_string(&path).ok()?;
        match serde_json::from_str::<Self>(&content) {
            Ok(cache) if cache.schema_version == Self::SCHEMA_VERSION => {
                let total = cache.total_entries();
                debug!(pr_number, repo, total, "Loaded review cache");
                Some(cache)
            }
            Ok(_) => {
                warn!("Review cache schema mismatch — ignoring stale cache");
                None
            }
            Err(e) => {
                warn!("Failed to parse review cache at {}: {e}", path.display());
                None
            }
        }
    }

    pub fn save(&self) {
        let path = cache_path(self.pr_number, &self.repo);
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                warn!("Failed to create cache dir {}: {e}", parent.display());
                return;
            }
        }
        match serde_json::to_string_pretty(self) {
            Ok(json) => match std::fs::write(&path, json) {
                Ok(()) => info!(pr = self.pr_number, repo = %self.repo, "Review cache saved"),
                Err(e) => warn!("Failed to write cache to {}: {e}", path.display()),
            },
            Err(e) => warn!("Failed to serialise review cache: {e}"),
        }
    }

    // ── Query ─────────────────────────────────────────────────────────────────

    /// Look up cached comments for `(agent_id, file_path)`.
    /// Returns `Some` only if the stored blob SHA matches `current_blob_sha`.
    pub fn get(
        &self,
        agent_id: &str,
        file_path: &str,
        current_blob_sha: &str,
    ) -> Option<&[CachedComment]> {
        let entry = self.entries.get(agent_id)?.get(file_path)?;
        if entry.blob_sha == current_blob_sha {
            Some(&entry.comments)
        } else {
            None
        }
    }

    /// Files for which the cache is valid for this agent (blob SHA unchanged).
    ///
    /// Returns `(hits, total_cached)` where `hits` is a list of file paths.
    pub fn hits_for_agent(
        &self,
        agent_id: &str,
        current_blobs: &HashMap<String, String>,
    ) -> Vec<String> {
        let Some(agent_map) = self.entries.get(agent_id) else {
            return Vec::new();
        };
        agent_map
            .iter()
            .filter(|(path, entry)| {
                // GENERAL_KEY is valid as long as the agent entry exists
                // (general comments don't have a blob SHA to compare)
                if path.as_str() == Self::GENERAL_KEY {
                    return true;
                }
                current_blobs
                    .get(*path)
                    .map(|s| s.as_str() == entry.blob_sha)
                    .unwrap_or(false)
            })
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// Collect all cached comments for `agent_id` that are still valid.
    pub fn valid_comments_for_agent(
        &self,
        agent_id: &str,
        current_blobs: &HashMap<String, String>,
    ) -> Vec<GeneratedComment> {
        let Some(agent_map) = self.entries.get(agent_id) else {
            return Vec::new();
        };
        agent_map
            .iter()
            .filter(|(path, entry)| {
                path.as_str() == Self::GENERAL_KEY
                    || current_blobs
                        .get(*path)
                        .map(|s| s.as_str() == entry.blob_sha)
                        .unwrap_or(false)
            })
            .flat_map(|(_, entry)| entry.comments.iter().map(|c| c.to_comment()))
            .collect()
    }

    // ── Mutation ──────────────────────────────────────────────────────────────

    /// Store agent results. Comments are grouped by file path; comments without
    /// a file_path are stored under `GENERAL_KEY`.
    pub fn put_agent_results(
        &mut self,
        agent_id: &str,
        comments: &[GeneratedComment],
        blob_shas: &HashMap<String, String>,
    ) {
        // Group comments by file path
        let mut by_file: HashMap<String, Vec<CachedComment>> = HashMap::new();
        for comment in comments {
            let key = comment
                .file_path
                .as_deref()
                .unwrap_or(Self::GENERAL_KEY)
                .to_string();
            by_file
                .entry(key)
                .or_default()
                .push(CachedComment::from_comment(comment));
        }

        // Also ensure files that had zero comments are cached (so we know
        // the agent ran clean on that file and don't re-run it next time).
        for path in blob_shas.keys() {
            by_file.entry(path.clone()).or_default();
        }

        let agent_map = self.entries.entry(agent_id.to_string()).or_default();
        for (file_path, cached_comments) in by_file {
            let blob_sha = if file_path == Self::GENERAL_KEY {
                "general".to_string()
            } else {
                blob_shas
                    .get(&file_path)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string())
            };
            agent_map.insert(
                file_path,
                FileCacheEntry {
                    blob_sha,
                    reviewed_at: Utc::now(),
                    comments: cached_comments,
                },
            );
        }
    }

    // ── Stats ─────────────────────────────────────────────────────────────────

    pub fn total_entries(&self) -> usize {
        self.entries.values().map(|m| m.len()).sum()
    }

    /// Number of cache hits across all agents given the current blob SHAs.
    pub fn hit_count(&self, current_blobs: &HashMap<String, String>) -> usize {
        self.entries
            .values()
            .flat_map(|m| m.iter())
            .filter(|(path, entry)| {
                path.as_str() == Self::GENERAL_KEY
                    || current_blobs
                        .get(*path)
                        .map(|s| s.as_str() == entry.blob_sha)
                        .unwrap_or(false)
            })
            .count()
    }
}

// ── Cleanup ───────────────────────────────────────────────────────────────────

/// Delete the cache file for a single PR (e.g. on manual "clear cache").
pub fn delete(pr_number: u64, repo: &str) {
    let path = cache_path(pr_number, repo);
    if path.exists() {
        match std::fs::remove_file(&path) {
            Ok(()) => info!(pr_number, repo, "Review cache deleted"),
            Err(e) => warn!("Failed to delete cache {}: {e}", path.display()),
        }
    }
}

/// Remove cache files for PRs that are no longer open.
///
/// Call this after loading the PR list. Deletes every `pr-{N}.json` in the
/// cache directory whose PR number is **not** in `open_numbers`.
pub fn prune_closed(repo: &str, open_numbers: &[u64]) {
    let dir = cache_dir(repo);
    let open_set: std::collections::HashSet<u64> = open_numbers.iter().copied().collect();

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return, // cache dir doesn't exist yet — nothing to prune
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(num_str) = name.strip_prefix("pr-").and_then(|s| s.strip_suffix(".json")) {
            if let Ok(n) = num_str.parse::<u64>() {
                if !open_set.contains(&n) {
                    match std::fs::remove_file(entry.path()) {
                        Ok(()) => info!(pr = n, repo, "Pruned cache for closed/merged PR"),
                        Err(e) => warn!("Failed to prune cache {}: {e}", entry.path().display()),
                    }
                }
            }
        }
    }
}

/// Remove cache files that haven't been updated in `max_age_days` days (TTL).
pub fn prune_stale(repo: &str, max_age_days: u64) {
    let dir = cache_dir(repo);
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(max_age_days * 86_400))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(meta) = std::fs::metadata(&path) {
            if let Ok(modified) = meta.modified() {
                if modified < cutoff {
                    match std::fs::remove_file(&path) {
                        Ok(()) => info!(repo, "Pruned stale cache {}", path.display()),
                        Err(e) => warn!("Failed to prune stale cache {}: {e}", path.display()),
                    }
                }
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Directory that holds all cache files for a given repo.
fn cache_dir(repo: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let repo_slug = repo.replace('/', "-");
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("prism")
        .join("cache")
        .join(repo_slug)
}

/// Build the on-disk path for the review cache.
fn cache_path(pr_number: u64, repo: &str) -> PathBuf {
    cache_dir(repo).join(format!("pr-{pr_number}.json"))
}

/// Extract git blob SHAs from a unified diff.
///
/// Parses `index <old_sha>..<new_sha>` lines that follow each `diff --git` header.
/// Returns a map of `file_path → new_blob_sha` (the current file version in the PR).
/// New files (old sha = `0000000`) and deleted files (new sha = `0000000`) are included.
pub fn extract_blob_shas(diff: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let mut current_file: Option<String> = None;

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            current_file = line
                .split(" b/")
                .nth(1)
                .map(|s| s.trim().to_string());
        } else if line.starts_with("index ") {
            // "index abc123..def456 100644"  or  "index abc123..def456"
            let sha_part = line["index ".len()..]
                .split_whitespace()
                .next()
                .unwrap_or("");
            if let (Some(path), Some(new_sha)) = (
                current_file.as_ref(),
                sha_part.split("..").nth(1),
            ) {
                // Exclude fully-zeroed SHAs (no object)
                if !new_sha.trim_start_matches('0').is_empty() {
                    result.insert(path.clone(), new_sha.to_string());
                }
            }
        }
    }

    result
}
