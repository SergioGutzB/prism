use std::collections::HashMap;
use std::sync::Arc;
use crate::github::models::PrDetails;
use crate::review::models::GeneratedComment;
use crate::tickets::models::Ticket;

/// Findings produced by a single specialist agent in Phase 1.
#[derive(Debug, Clone)]
pub struct AgentFinding {
    pub agent_id: String,
    pub agent_name: String,
    pub agent_icon: String,
    pub comments: Vec<GeneratedComment>,
}

// ── Phase-0 Objective Analysis ────────────────────────────────────────────────

/// How well the PR implementation aligns with the stated ticket objectives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectiveAlignment {
    Aligned,
    Partial,
    Misaligned,
}

/// Structured output produced by the Phase-0 objective-validator agent.
/// Injected into all subsequent agent prompts.
#[derive(Debug, Clone)]
pub struct ObjectiveAnalysis {
    pub stated_objectives: String,
    pub implementation_summary: String,
    pub alignment: ObjectiveAlignment,
    pub gaps: Vec<String>,
    pub overall_assessment: String,
}

impl ObjectiveAnalysis {
    /// Format as a compact Markdown block for injection into later agent prompts.
    pub fn as_context_text(&self) -> String {
        let alignment_str = match self.alignment {
            ObjectiveAlignment::Aligned    => "Aligned",
            ObjectiveAlignment::Partial    => "Partial",
            ObjectiveAlignment::Misaligned => "Misaligned",
        };
        let mut out = format!(
            "## Objective Analysis\n\n\
             **Stated Objectives:** {}\n\n\
             **Implementation Summary:** {}\n\n\
             **Alignment:** {}\n\n\
             **Overall Assessment:** {}\n",
            self.stated_objectives,
            self.implementation_summary,
            alignment_str,
            self.overall_assessment,
        );
        if !self.gaps.is_empty() {
            out.push_str("\n**Gaps / Concerns:**\n");
            for gap in &self.gaps {
                out.push_str(&format!("- {gap}\n"));
            }
        }
        out
    }
}

// ── ReviewContext ─────────────────────────────────────────────────────────────

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
    /// Findings from Phase-1 specialist agents, injected by the orchestrator
    /// before running Phase-2 synthesis agents. Empty for Phase-1 agents.
    pub prior_findings: Vec<AgentFinding>,
    /// Analysis from the Phase-0 objective-validator agent. Injected into all
    /// Phase-1 and Phase-2 agent prompts so they understand ticket alignment.
    pub objective_analysis: Option<ObjectiveAnalysis>,
    /// `"owner/repo"` — used as the cache namespace.
    pub repo_slug: String,
    /// Git blob SHAs extracted from the diff: `file_path → blob_sha`.
    /// Used by the orchestrator to decide which files need re-reviewing.
    pub blob_shas: HashMap<String, String>,
    /// When non-empty, `prepare_diff` restricts the diff to only these file
    /// paths, overriding the agent's own `include_patterns`.
    /// Set by the orchestrator for partial cache hits.
    pub cache_skip_paths: Vec<String>,
    /// Detected frameworks/build-tools inferred from file extensions and manifest names.
    pub detected_frameworks: Vec<String>,
    /// Contents of CONTRIBUTING.md / PR template fetched from the repo (if available).
    pub project_conventions: Option<String>,
}

impl ReviewContext {
    /// Build a `ReviewContext` from GitHub PR details + raw diff text.
    ///
    /// `repo_slug` should be `"owner/repo"` — used for cache lookup.
    pub fn from_pr(pr: &PrDetails, diff: &str, ticket: Option<Ticket>, repo_slug: &str) -> Self {
        let raw_diff: Arc<str> = Arc::from(diff);
        let diff_stats = DiffStats::from_diff(diff);
        let changed_files = parse_changed_files(diff);
        let blob_shas = crate::review::cache::extract_blob_shas(diff);

        let detected_frameworks = detect_frameworks(&changed_files, diff);

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
            prior_findings: Vec::new(),
            objective_analysis: None,
            repo_slug: repo_slug.to_string(),
            blob_shas,
            cache_skip_paths: Vec::new(),
            detected_frameworks,
            project_conventions: None,
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
                let gen_tag = if f.is_generated { " [generated]" } else { "" };
                format!(
                    "[{}] {} (+{} -{}){gen_tag}",
                    status_str, f.path, f.additions, f.deletions
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Prepare the diff for LLM consumption.
    ///
    /// Steps:
    /// 1. Split the diff into per-file sections.
    /// 2. Filter out files matching `global_exclude` or `agent_exclude` patterns.
    ///    If `agent_include` is non-empty, keep only files matching those patterns.
    /// 3. Truncate at file boundaries so the token budget is never exceeded by a
    ///    broken-hunk mid-truncation.
    ///
    /// Returns a [`PreparedDiff`] that includes metadata about what was filtered.
    pub fn prepare_diff(
        &self,
        global_exclude: &[String],
        agent_exclude: &[String],
        agent_include: &[String],
        max_tokens: u32,
    ) -> PreparedDiff {
        let sections = split_diff_by_file(&self.raw_diff);

        // If the orchestrator set cache_skip_paths, those files are already
        // cached and must be excluded from the diff sent to the LLM.
        let effective_exclude: Vec<String> = global_exclude
            .iter()
            .chain(self.cache_skip_paths.iter())
            .cloned()
            .collect();

        let mut included_sections: Vec<(String, String)> = Vec::new();
        let mut excluded_names: Vec<String> = Vec::new();

        for (path, section) in sections {
            if should_include_file(&path, &effective_exclude, agent_exclude, agent_include) {
                included_sections.push((path, section));
            } else {
                excluded_names.push(path);
            }
        }

        // ~3.5 chars/token is empirically closer to reality than 4 for code.
        let max_chars = (max_tokens as usize) * 35 / 10;
        let mut diff = String::with_capacity(max_chars.min(self.raw_diff.len()));
        let mut truncated_names: Vec<String> = Vec::new();
        let mut files_included: usize = 0;

        for (path, section) in included_sections {
            if diff.len() + section.len() <= max_chars {
                // Enrich each file section with semantic context labels
                let enriched = enrich_diff_with_context(&section);
                diff.push_str(&enriched);
                files_included += 1;
            } else {
                truncated_names.push(path);
            }
        }

        PreparedDiff {
            diff,
            files_included,
            files_excluded: excluded_names.len(),
            files_truncated: truncated_names.len(),
            excluded_names,
            truncated_names,
        }
    }

    /// Format the Phase-0 objective analysis for injection into later agent prompts.
    /// Returns `None` when no objective analysis is available.
    pub fn objective_text(&self) -> Option<String> {
        self.objective_analysis.as_ref().map(|a| a.as_context_text())
    }

    /// Format the Phase-1 specialist findings as a Markdown section for injection
    /// into a synthesis agent's prompt. Returns `None` when there are no findings
    /// (so the section is omitted entirely and no tokens are wasted).
    pub fn findings_text(&self) -> Option<String> {
        if self.prior_findings.is_empty() {
            return None;
        }

        let mut out = String::new();
        let total: usize = self.prior_findings.iter().map(|f| f.comments.len()).sum();

        if total == 0 {
            // All specialist agents ran but found nothing — still worth noting
            out.push_str("All specialist reviewers found no issues in this PR.\n");
            return Some(out);
        }

        for finding in &self.prior_findings {
            let count = finding.comments.len();
            out.push_str(&format!(
                "### {} {} ({} finding(s))\n\n",
                finding.agent_icon, finding.agent_name, count
            ));

            if count == 0 {
                out.push_str("*(No issues found)*\n\n");
                continue;
            }

            for comment in &finding.comments {
                use crate::review::models::Severity;
                let sev = match comment.severity {
                    Severity::Critical => "critical",
                    Severity::Warning  => "warning",
                    Severity::Suggestion => "suggestion",
                    Severity::Praise   => "praise",
                };
                let location = match (&comment.file_path, comment.line) {
                    (Some(f), Some(l)) => format!("`{}:{}`", f, l),
                    (Some(f), None)    => format!("`{}`", f),
                    _                  => "(general)".to_string(),
                };
                out.push_str(&format!(
                    "- **[{sev}]** {location} — {}\n",
                    comment.effective_body()
                ));
            }
            out.push('\n');
        }

        Some(out)
    }

    /// Simple truncation by character count (kept as fallback).
    pub fn truncated_diff(&self, max_tokens: u32) -> &str {
        let max_chars = (max_tokens as usize) * 4;
        let diff = &*self.raw_diff;
        if diff.len() <= max_chars { diff } else { &diff[..max_chars] }
    }
}

// ── PreparedDiff ─────────────────────────────────────────────────────────────

/// The result of `ReviewContext::prepare_diff` — a filtered, boundary-truncated
/// diff string ready to be embedded into an LLM prompt.
pub struct PreparedDiff {
    /// Filtered and truncated unified diff text.
    pub diff: String,
    /// Number of files whose sections are present in `diff`.
    pub files_included: usize,
    /// Number of files removed by exclude/include pattern filters.
    pub files_excluded: usize,
    /// Number of files dropped because the token budget was exhausted.
    pub files_truncated: usize,
    /// Paths of excluded files (for the header note).
    pub excluded_names: Vec<String>,
    /// Paths of truncated files (for the header note).
    pub truncated_names: Vec<String>,
}

impl PreparedDiff {
    /// Build a one-line note describing what was filtered / truncated.
    /// Returns `None` when nothing was omitted.
    pub fn header_note(&self) -> Option<String> {
        if self.files_excluded == 0 && self.files_truncated == 0 {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        if self.files_excluded > 0 {
            let preview = self.excluded_names.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
            let more = if self.excluded_names.len() > 5 {
                format!(" +{} more", self.excluded_names.len() - 5)
            } else {
                String::new()
            };
            parts.push(format!("{} file(s) excluded by pattern: {}{}", self.files_excluded, preview, more));
        }
        if self.files_truncated > 0 {
            let preview = self.truncated_names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
            let more = if self.truncated_names.len() > 3 {
                format!(" +{} more", self.truncated_names.len() - 3)
            } else {
                String::new()
            };
            parts.push(format!("{} file(s) omitted (token limit): {}{}", self.files_truncated, preview, more));
        }
        Some(parts.join("; "))
    }

    /// Estimated tokens for the assembled diff string (~3.5 chars/token).
    pub fn estimated_tokens(&self) -> usize {
        self.diff.len() * 10 / 35
    }
}

// ── Semantic Diff Analysis ───────────────────────────────────────────────────

/// Extract the function or class name from a diff hunk header (the text after @@ ... @@).
/// Example: "@@ -10,5 +10,6 @@ fn my_function() {" -> Some("fn my_function()")
fn get_context_from_hunk_header(header: &str) -> Option<String> {
    if let Some(idx) = header.find("@@") {
        let after_at = &header[idx + 2..];
        if let Some(end_idx) = after_at.find("@@") {
            let context = &after_at[end_idx + 2..].trim();
            if !context.is_empty() {
                return Some(context.to_string());
            }
        }
    }
    None
}

/// Process a raw diff and inject semantic context labels before each hunk.
/// This helps the LLM understand WHICH function or class is being modified.
pub fn enrich_diff_with_context(diff: &str) -> String {
    let mut enriched = String::with_capacity(diff.len() + 1024);
    
    for line in diff.lines() {
        if line.starts_with("@@") {
            if let Some(ctx) = get_context_from_hunk_header(line) {
                enriched.push_str(&format!("\n[Context: {}]\n", ctx));
            }
        }
        enriched.push_str(line);
        enriched.push('\n');
    }
    
    enriched
}

// ── Diff splitting ────────────────────────────────────────────────────────────

/// Split a unified diff into per-file `(path, section_text)` pairs.
///
/// Each section includes its own `diff --git` header line through everything
/// up to (but not including) the next `diff --git` line.
pub fn split_diff_by_file(diff: &str) -> Vec<(String, String)> {
    let mut result: Vec<(String, String)> = Vec::new();
    let mut current_path: Option<String> = None;
    let mut section_start: usize = 0;
    let mut byte_pos: usize = 0;

    for line in diff.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            if let Some(path) = current_path.take() {
                result.push((path, diff[section_start..byte_pos].to_owned()));
            }
            section_start = byte_pos;
            // "diff --git a/foo b/foo" → take everything after the last " b/"
            current_path = line.trim_end()
                .split(" b/")
                .nth(1)
                .map(str::to_owned);
        }
        byte_pos += line.len();
    }
    if let Some(path) = current_path {
        result.push((path, diff[section_start..].to_owned()));
    }
    result
}

// ── Glob pattern matching ─────────────────────────────────────────────────────

/// Return `true` if `path` should be kept given the exclude and include lists.
///
/// Logic:
/// - If `include` is non-empty, the file must match at least one include pattern.
/// - The file must not match any pattern in `global_exclude` or `agent_exclude`.
fn should_include_file(
    path: &str,
    global_exclude: &[String],
    agent_exclude: &[String],
    include: &[String],
) -> bool {
    if !include.is_empty() && !include.iter().any(|p| file_matches_pattern(p, path)) {
        return false;
    }
    if global_exclude.iter().any(|p| file_matches_pattern(p, path)) {
        return false;
    }
    if agent_exclude.iter().any(|p| file_matches_pattern(p, path)) {
        return false;
    }
    true
}

/// Match a glob pattern against a file path.
///
/// `*` matches any run of characters (including `/`).
/// The pattern is tried against the full path **and** against each sub-path
/// that starts after a `/` boundary, so `dist/*` matches both
/// `dist/bundle.js` and `pkg/dist/bundle.js`.
pub fn file_matches_pattern(pattern: &str, path: &str) -> bool {
    let pat = pattern.as_bytes();
    let p = path.as_bytes();
    if glob_bytes(pat, p) {
        return true;
    }
    // Try each sub-path
    for (i, &b) in p.iter().enumerate() {
        if b == b'/' && i + 1 < p.len() {
            if glob_bytes(pat, &p[i + 1..]) {
                return true;
            }
        }
    }
    false
}

/// Recursive byte-level glob matcher. Only `*` (matches anything) is supported.
fn glob_bytes(pat: &[u8], text: &[u8]) -> bool {
    match pat.first() {
        None => text.is_empty(),
        Some(b'*') => {
            // '*' matches zero or more characters
            (0..=text.len()).any(|i| glob_bytes(&pat[1..], &text[i..]))
        }
        Some(&pc) => {
            matches!(text.first(), Some(&tc) if tc == pc)
                && glob_bytes(&pat[1..], &text[1..])
        }
    }
}

// ── Changed-file parsing ──────────────────────────────────────────────────────

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
    let mut status = FileStatus::Modified;

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            if let Some(path) = current_path.take() {
                let is_generated = is_generated_file(&path);
                files.push(ChangedFile {
                    path,
                    status,
                    additions,
                    deletions,
                    diff_hunk: std::mem::take(&mut hunk_buf),
                    is_generated,
                });
            }
            additions = 0;
            deletions = 0;
            status = FileStatus::Modified;
            if let Some(b_part) = line.split(" b/").nth(1) {
                current_path = Some(b_part.to_string());
            }
        } else if line.starts_with("new file mode") {
            status = FileStatus::Added;
        } else if line.starts_with("deleted file mode") {
            status = FileStatus::Deleted;
        } else if line.starts_with("rename from ") {
            if let Some(from) = line.strip_prefix("rename from ") {
                status = FileStatus::Renamed { from: from.to_string() };
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

    if let Some(path) = current_path.take() {
        let is_generated = is_generated_file(&path);
        files.push(ChangedFile {
            path,
            status,
            additions,
            deletions,
            diff_hunk: hunk_buf,
            is_generated,
        });
    }

    files
}

/// Heuristic: detect generated/vendor files by path patterns.
fn is_generated_file(path: &str) -> bool {
    let generated_patterns = [
        "*.lock",
        "*.generated.*",
        "*.min.js",
        "*.min.css",
        "dist/*",
        "build/*",
        "vendor/*",
        "node_modules/*",
        "__generated__/*",
    ];
    generated_patterns.iter().any(|p| file_matches_pattern(p, path))
}

impl DiffStats {
    pub fn from_diff(diff: &str) -> Self {
        let mut additions: u32 = 0;
        let mut deletions: u32 = 0;
        let mut files: u32 = 0;

        for line in diff.lines() {
            if line.starts_with("diff --git") {
                files += 1;
            } else if line.starts_with('+') && !line.starts_with("+++ ") {
                additions += 1;
            } else if line.starts_with('-') && !line.starts_with("--- ") {
                deletions += 1;
            }
        }

        // ~3.5 chars/token for code is more accurate than 4
        let estimated_tokens = (diff.len() as u32) * 10 / 35;

        Self {
            total_additions: additions,
            total_deletions: deletions,
            files_changed: files,
            estimated_tokens,
        }
    }
}

// ── Framework detection ───────────────────────────────────────────────────────

/// Infer build tools and frameworks from file extensions and manifest file names
/// present in the diff. Returns a deduplicated, sorted list of detected technologies.
pub fn detect_frameworks(files: &[ChangedFile], diff: &str) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut detected: BTreeSet<String> = BTreeSet::new();

    // Map of manifest/config file names → technology label
    let manifest_map: &[(&str, &str)] = &[
        ("Cargo.toml",         "Cargo"),
        ("Cargo.lock",         "Cargo"),
        ("package.json",       "Node.js / npm"),
        ("package-lock.json",  "Node.js / npm"),
        ("yarn.lock",          "Node.js / Yarn"),
        ("pnpm-lock.yaml",     "Node.js / pnpm"),
        ("pyproject.toml",     "Python / Poetry"),
        ("setup.py",           "Python / setuptools"),
        ("requirements.txt",   "Python / pip"),
        ("Pipfile",            "Python / Pipenv"),
        ("go.mod",             "Go / modules"),
        ("go.sum",             "Go / modules"),
        ("pom.xml",            "Java / Maven"),
        ("build.gradle",       "Java / Gradle"),
        ("build.gradle.kts",   "Kotlin / Gradle"),
        ("Gemfile",            "Ruby / Bundler"),
        ("composer.json",      "PHP / Composer"),
        ("pubspec.yaml",       "Dart / Flutter"),
        ("mix.exs",            "Elixir / Mix"),
        ("Makefile",           "Make"),
        ("CMakeLists.txt",     "C/C++ / CMake"),
        ("*.csproj",           "C# / .NET"),
        ("*.sln",              "C# / .NET"),
        ("tsconfig.json",      "TypeScript"),
        ("next.config.*",      "Next.js"),
        ("nuxt.config.*",      "Nuxt.js"),
        ("vite.config.*",      "Vite"),
        ("webpack.config.*",   "Webpack"),
        ("tailwind.config.*",  "Tailwind CSS"),
        ("docker-compose.*",   "Docker Compose"),
        ("Dockerfile",         "Docker"),
    ];

    // Map of file extensions → language label
    let ext_map: &[(&str, &str)] = &[
        ("rs",    "Rust"),
        ("ts",    "TypeScript"),
        ("tsx",   "TypeScript / React"),
        ("js",    "JavaScript"),
        ("jsx",   "JavaScript / React"),
        ("mjs",   "JavaScript"),
        ("py",    "Python"),
        ("go",    "Go"),
        ("java",  "Java"),
        ("kt",    "Kotlin"),
        ("swift", "Swift"),
        ("rb",    "Ruby"),
        ("php",   "PHP"),
        ("cs",    "C#"),
        ("cpp",   "C++"),
        ("cc",    "C++"),
        ("c",     "C"),
        ("h",     "C/C++"),
        ("dart",  "Dart"),
        ("ex",    "Elixir"),
        ("exs",   "Elixir"),
        ("hs",    "Haskell"),
        ("scala", "Scala"),
        ("clj",   "Clojure"),
        ("vue",   "Vue.js"),
        ("svelte","Svelte"),
    ];

    for file in files {
        let name = std::path::Path::new(&file.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        // Check manifest names (exact match on filename)
        for (manifest, label) in manifest_map {
            if manifest.starts_with('*') {
                // Glob suffix: e.g. "*.csproj" → check extension
                let ext = manifest.trim_start_matches("*.");
                if name.ends_with(&format!(".{}", ext)) {
                    detected.insert(label.to_string());
                }
            } else if name == *manifest || file.path == *manifest {
                detected.insert(label.to_string());
            }
            // Partial match for config files like "next.config.*"
            if manifest.ends_with(".*") {
                let prefix = manifest.trim_end_matches(".*");
                if name.starts_with(prefix) {
                    detected.insert(label.to_string());
                }
            }
        }

        // Check file extension
        if let Some(ext) = std::path::Path::new(&file.path).extension().and_then(|e| e.to_str()) {
            for (file_ext, label) in ext_map {
                if ext.eq_ignore_ascii_case(file_ext) {
                    detected.insert(label.to_string());
                }
            }
        }
    }

    // Also scan diff headers for manifest files not in changed_files list
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ b/") {
            let name = std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            for (manifest, label) in manifest_map {
                if !manifest.contains('*') && (name == *manifest) {
                    detected.insert(label.to_string());
                }
            }
        }
    }

    detected.into_iter().collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches_extension() {
        assert!(file_matches_pattern("*.lock", "Cargo.lock"));
        assert!(file_matches_pattern("*.lock", "yarn.lock"));
        assert!(file_matches_pattern("*.lock", "pkg/Cargo.lock"));
        assert!(!file_matches_pattern("*.lock", "lockfile.txt"));
    }

    #[test]
    fn glob_matches_directory() {
        assert!(file_matches_pattern("dist/*", "dist/bundle.js"));
        assert!(file_matches_pattern("dist/*", "frontend/dist/bundle.js"));
        assert!(!file_matches_pattern("dist/*", "distribution/foo.js"));
    }

    #[test]
    fn glob_matches_double_wildcard() {
        assert!(file_matches_pattern("*.generated.*", "api.generated.ts"));
        assert!(file_matches_pattern("*.generated.*", "src/api.generated.ts"));
        assert!(!file_matches_pattern("*.generated.*", "api_generated_ts"));
    }

    #[test]
    fn glob_exact_filename() {
        assert!(file_matches_pattern("package-lock.json", "package-lock.json"));
        assert!(file_matches_pattern("package-lock.json", "frontend/package-lock.json"));
        assert!(!file_matches_pattern("package-lock.json", "package-lock.json.bak"));
    }

    #[test]
    fn split_diff_parses_correctly() {
        let diff = "\
diff --git a/src/main.rs b/src/main.rs
index abc..def 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
 fn main() {}
diff --git a/Cargo.lock b/Cargo.lock
index 123..456 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -1,2 +1,3 @@
 # lock file
";
        let sections = split_diff_by_file(diff);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].0, "src/main.rs");
        assert_eq!(sections[1].0, "Cargo.lock");
        assert!(sections[0].1.starts_with("diff --git a/src/main.rs"));
        assert!(sections[1].1.starts_with("diff --git a/Cargo.lock"));
    }

    // ── split_diff edge cases ─────────────────────────────────────────────────

    #[test]
    fn split_diff_empty_string() {
        let sections = split_diff_by_file("");
        assert!(sections.is_empty());
    }

    #[test]
    fn split_diff_single_file() {
        let diff = "diff --git a/foo.rs b/foo.rs\n+line\n";
        let sections = split_diff_by_file(diff);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, "foo.rs");
        assert!(sections[0].1.contains("+line"));
    }

    #[test]
    fn split_diff_file_with_spaces_in_path() {
        let diff = "diff --git a/src/my file.rs b/src/my file.rs\n+code\n";
        let sections = split_diff_by_file(diff);
        // Path after last " b/" should be "src/my file.rs"
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, "src/my file.rs");
    }

    #[test]
    fn split_diff_no_trailing_newline() {
        let diff = "diff --git a/a.rs b/a.rs\n+x";
        let sections = split_diff_by_file(diff);
        assert_eq!(sections.len(), 1);
        assert!(sections[0].1.contains("+x"));
    }

    // ── parse_changed_files ───────────────────────────────────────────────────

    #[test]
    fn parse_files_counts_additions_and_deletions() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
index abc..def 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@
 unchanged
+added line
+another add
-removed line
";
        let files = parse_changed_files(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].additions, 2);
        assert_eq!(files[0].deletions, 1);
    }

    #[test]
    fn parse_files_detects_new_file() {
        let diff = "\
diff --git a/src/new.rs b/src/new.rs
new file mode 100644
+fn foo() {}
";
        let files = parse_changed_files(diff);
        assert_eq!(files.len(), 1);
        assert!(matches!(files[0].status, FileStatus::Added));
    }

    #[test]
    fn parse_files_detects_deleted_file() {
        let diff = "\
diff --git a/src/old.rs b/src/old.rs
deleted file mode 100644
-fn old() {}
";
        let files = parse_changed_files(diff);
        assert_eq!(files.len(), 1);
        assert!(matches!(files[0].status, FileStatus::Deleted));
    }

    #[test]
    fn parse_files_detects_rename() {
        let diff = "\
diff --git a/src/a.rs b/src/b.rs
rename from src/a.rs
rename to src/b.rs
";
        let files = parse_changed_files(diff);
        assert_eq!(files.len(), 1);
        assert!(matches!(&files[0].status, FileStatus::Renamed { from } if from == "src/a.rs"));
    }

    #[test]
    fn parse_files_marks_lock_file_as_generated() {
        let diff = "diff --git a/Cargo.lock b/Cargo.lock\n+[package]\n";
        let files = parse_changed_files(diff);
        assert_eq!(files.len(), 1);
        assert!(files[0].is_generated, "Cargo.lock should be marked as generated");
    }

    #[test]
    fn parse_files_does_not_count_diff_header_lines() {
        // Lines starting with +++ and --- are diff headers, not additions/deletions
        let diff = "\
diff --git a/foo.py b/foo.py
--- a/foo.py
+++ b/foo.py
@@ -1 +1 @@
-old
+new
";
        let files = parse_changed_files(diff);
        assert_eq!(files[0].additions, 1);
        assert_eq!(files[0].deletions, 1);
    }

    // ── DiffStats ─────────────────────────────────────────────────────────────

    #[test]
    fn diff_stats_empty_diff() {
        let s = DiffStats::from_diff("");
        assert_eq!(s.files_changed, 0);
        assert_eq!(s.total_additions, 0);
        assert_eq!(s.total_deletions, 0);
    }

    #[test]
    fn diff_stats_counts_correctly() {
        let diff = "\
diff --git a/a.ts b/a.ts
+++ b/a.ts
+line1
+line2
-removed
diff --git a/b.ts b/b.ts
+line3
";
        let s = DiffStats::from_diff(diff);
        assert_eq!(s.files_changed, 2);
        assert_eq!(s.total_additions, 3);
        assert_eq!(s.total_deletions, 1);
    }

    // ── detect_frameworks ─────────────────────────────────────────────────────

    fn make_changed_file(path: &str) -> ChangedFile {
        ChangedFile {
            path: path.to_string(),
            status: FileStatus::Modified,
            additions: 0,
            deletions: 0,
            diff_hunk: String::new(),
            is_generated: false,
        }
    }

    #[test]
    fn detect_rust_from_cargo_toml() {
        let files = vec![make_changed_file("Cargo.toml")];
        let result = detect_frameworks(&files, "");
        assert!(result.contains(&"Cargo".to_string()));
    }

    #[test]
    fn detect_typescript_from_extension() {
        let files = vec![make_changed_file("src/app.ts")];
        let result = detect_frameworks(&files, "");
        assert!(result.contains(&"TypeScript".to_string()));
    }

    #[test]
    fn detect_react_from_tsx() {
        let files = vec![make_changed_file("src/App.tsx")];
        let result = detect_frameworks(&files, "");
        assert!(result.contains(&"TypeScript / React".to_string()));
    }

    #[test]
    fn detect_nextjs_from_config() {
        let files = vec![make_changed_file("next.config.js")];
        let result = detect_frameworks(&files, "");
        assert!(result.contains(&"Next.js".to_string()));
    }

    #[test]
    fn detect_multiple_frameworks() {
        let files = vec![
            make_changed_file("Cargo.toml"),
            make_changed_file("package.json"),
            make_changed_file("src/lib.rs"),
        ];
        let result = detect_frameworks(&files, "");
        assert!(result.contains(&"Cargo".to_string()));
        assert!(result.contains(&"Node.js / npm".to_string()));
        assert!(result.contains(&"Rust".to_string()));
    }

    #[test]
    fn detect_frameworks_from_diff_headers() {
        // Files not in changed_files but visible in diff +++ headers
        let diff = "+++ b/go.mod\n+module myapp\n";
        let result = detect_frameworks(&[], diff);
        assert!(result.contains(&"Go / modules".to_string()));
    }

    #[test]
    fn detect_frameworks_deduplicates() {
        // Two .ts files should only produce "TypeScript" once
        let files = vec![
            make_changed_file("src/a.ts"),
            make_changed_file("src/b.ts"),
        ];
        let result = detect_frameworks(&files, "");
        let ts_count = result.iter().filter(|s| *s == "TypeScript").count();
        assert_eq!(ts_count, 1);
    }

    #[test]
    fn detect_frameworks_empty_diff() {
        let result = detect_frameworks(&[], "");
        assert!(result.is_empty());
    }

    // ── glob edge cases ───────────────────────────────────────────────────────

    #[test]
    fn glob_empty_pattern_matches_only_empty_path() {
        assert!(file_matches_pattern("", ""));
        assert!(!file_matches_pattern("", "foo"));
    }

    #[test]
    fn glob_star_only_matches_anything() {
        assert!(file_matches_pattern("*", "foo.rs"));
        assert!(file_matches_pattern("*", ""));
        assert!(file_matches_pattern("*", "a/b/c.ts"));
    }

    #[test]
    fn glob_min_js_matches_in_subdirectory() {
        assert!(file_matches_pattern("*.min.js", "dist/bundle.min.js"));
        assert!(!file_matches_pattern("*.min.js", "dist/bundle.js"));
    }

    #[test]
    fn prepare_diff_excludes_patterns() {
        let diff = "\
diff --git a/src/main.rs b/src/main.rs
index abc..def 100644
+fn main() {}
diff --git a/Cargo.lock b/Cargo.lock
index 123..456 100644
+[lock content]
";
        // Build a minimal ReviewContext directly without going through from_pr
        let ctx = ReviewContext {
            pr_number: 1,
            pr_title: "Test".into(),
            pr_description: String::new(),
            pr_author: "user".into(),
            base_branch: "main".into(),
            head_branch: "feature".into(),
            pr_url: String::new(),
            raw_diff: std::sync::Arc::from(diff),
            changed_files: vec![],
            diff_stats: DiffStats::default(),
            ticket: None,
            repo_language: None,
            prior_findings: vec![],
            objective_analysis: None,
            repo_slug: "owner/repo".to_string(),
            blob_shas: std::collections::HashMap::new(),
            cache_skip_paths: vec![],
            detected_frameworks: vec![],
            project_conventions: None,
        };
        let prepared = ctx.prepare_diff(
            &["*.lock".to_string()],
            &[],
            &[],
            8000,
        );
        assert_eq!(prepared.files_included, 1);
        assert_eq!(prepared.files_excluded, 1);
        assert!(prepared.diff.contains("src/main.rs"));
        assert!(!prepared.diff.contains("Cargo.lock"));
    }
}
