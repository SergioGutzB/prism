use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};

use crate::agents::context::{AgentFinding, ObjectiveAnalysis, ReviewContext};
use crate::agents::models::{AgentDefinition, AgentStatus};
use crate::agents::runner::AgentRunner;
use crate::config::AppConfig;
use crate::review::cache::ReviewCache;

/// Events emitted by the orchestrator during a run.
#[derive(Debug)]
pub struct AgentUpdate {
    pub agent_id: String,
    pub status: AgentStatus,
}

pub struct Orchestrator {
    runner: Arc<AgentRunner>,
    concurrency: usize,
}

impl Orchestrator {
    pub fn new(config: AppConfig) -> Self {
        let concurrency = config.agents.concurrency.max(1);
        Self {
            runner: Arc::new(AgentRunner::new(config)),
            concurrency,
        }
    }

    /// Run all enabled agents in three collaborative phases with cache support.
    ///
    /// **Phase 0** — Objective-validator agents run first (sequentially).
    ///
    /// **Phase 1** — Specialist agents run concurrently. For each agent the
    /// orchestrator checks the on-disk review cache:
    ///   - **Full hit** (all files cached) → emit `Skipped`, reuse cached comments.
    ///   - **Partial hit** (some files changed) → set `cache_skip_paths` so the
    ///     agent only processes changed files; merge new + cached comments.
    ///   - **Miss** (no cache or all files changed) → run normally.
    ///   After every agent its results are written back to the cache.
    ///
    /// **Phase 2** — Synthesis agents receive all Phase-1 findings (cached + new).
    ///
    /// The cache is saved to disk once all phases complete.
    pub fn run_all(
        &self,
        agents: Vec<AgentDefinition>,
        ctx: ReviewContext,
    ) -> mpsc::Receiver<AgentUpdate> {
        let (tx, rx) = mpsc::channel::<AgentUpdate>(256);
        let runner = Arc::clone(&self.runner);
        let concurrency = self.concurrency;

        // Load (or create) the review cache for this PR
        let cache = ReviewCache::load(ctx.pr_number, &ctx.repo_slug)
            .unwrap_or_else(|| ReviewCache::new(ctx.pr_number, &ctx.repo_slug));
        let cache = Arc::new(Mutex::new(cache));

        let ctx = Arc::new(ctx);

        tokio::spawn(async move {
            // ── Separate disabled / phase_zero / specialists / synthesis ───
            let mut disabled: Vec<AgentDefinition> = Vec::new();
            let mut phase_zero: Vec<AgentDefinition> = Vec::new();
            let mut specialists: Vec<AgentDefinition> = Vec::new();
            let mut synthesis: Vec<AgentDefinition> = Vec::new();

            for agent in agents {
                if !agent.agent.enabled {
                    disabled.push(agent);
                } else if agent.agent.phase_zero {
                    phase_zero.push(agent);
                } else if agent.agent.synthesis {
                    synthesis.push(agent);
                } else {
                    specialists.push(agent);
                }
            }

            // Immediately mark disabled agents
            for agent in disabled {
                let _ = tx.send(AgentUpdate {
                    agent_id: agent.agent.id.clone(),
                    status: AgentStatus::Disabled,
                }).await;
            }

            // Emit Pending for all enabled agents
            for agent in phase_zero.iter().chain(specialists.iter()).chain(synthesis.iter()) {
                let _ = tx.send(AgentUpdate {
                    agent_id: agent.agent.id.clone(),
                    status: AgentStatus::Pending,
                }).await;
            }

            // ── Phase 0: objective validator ───────────────────────────────
            let objective_analysis = run_phase_zero(
                phase_zero,
                Arc::clone(&ctx),
                Arc::clone(&runner),
                &tx,
            ).await;

            // Enrich context with objective analysis for Phase 1 + 2
            let ctx1: Arc<ReviewContext> = if objective_analysis.is_some() {
                let mut c = (*ctx).clone();
                c.objective_analysis = objective_analysis.clone();
                Arc::new(c)
            } else {
                Arc::clone(&ctx)
            };

            // ── Phase 1: specialist agents (cache-aware) ───────────────────
            let phase1_findings = run_phase_cached(
                specialists,
                Arc::clone(&ctx1),
                Arc::clone(&runner),
                concurrency,
                Arc::clone(&cache),
                &tx,
            ).await;

            // ── Phase 2: synthesis agents with enriched context ────────────
            if !synthesis.is_empty() {
                let mut ctx2 = (*ctx1).clone();
                ctx2.prior_findings = phase1_findings;
                let ctx2 = Arc::new(ctx2);

                run_phase(synthesis, ctx2, Arc::clone(&runner), concurrency, &tx).await;
            }

            // Persist the updated cache to disk
            cache.lock().await.save();

            info!("All agents completed");
        });

        rx
    }
}

// ── Phase 0 ───────────────────────────────────────────────────────────────────

async fn run_phase_zero(
    agents: Vec<AgentDefinition>,
    ctx: Arc<ReviewContext>,
    runner: Arc<AgentRunner>,
    tx: &mpsc::Sender<AgentUpdate>,
) -> Option<ObjectiveAnalysis> {
    let mut last_analysis: Option<ObjectiveAnalysis> = None;

    for agent in agents {
        let agent_id = agent.agent.id.clone();
        debug!(agent_id = %agent_id, "Phase-0 agent starting");

        let _ = tx.send(AgentUpdate {
            agent_id: agent_id.clone(),
            status: AgentStatus::Running { started_at: chrono::Utc::now() },
        }).await;

        let (status, analysis) = runner.run_objective(&agent, &ctx).await;

        if let AgentStatus::Done { ref comments, elapsed_ms, .. } = status {
            info!(agent_id = %agent_id, comment_count = comments.len(),
                  elapsed_ms, "Phase-0 agent done");
        } else if let AgentStatus::Failed { ref error } = status {
            error!(agent_id = %agent_id, error = %error, "Phase-0 agent failed");
        }

        if let Some(a) = analysis {
            last_analysis = Some(a);
        }

        let _ = tx.send(AgentUpdate { agent_id, status }).await;
    }

    last_analysis
}

// ── Phase 1 (cache-aware) ─────────────────────────────────────────────────────

/// Run specialist agents concurrently, consulting the cache before each run.
async fn run_phase_cached(
    agents: Vec<AgentDefinition>,
    ctx: Arc<ReviewContext>,
    runner: Arc<AgentRunner>,
    concurrency: usize,
    cache: Arc<Mutex<ReviewCache>>,
    tx: &mpsc::Sender<AgentUpdate>,
) -> Vec<AgentFinding> {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut join_set = JoinSet::new();

    for agent in agents {
        let permit     = Arc::clone(&semaphore);
        let runner     = Arc::clone(&runner);
        let ctx        = Arc::clone(&ctx);
        let cache      = Arc::clone(&cache);
        let tx         = tx.clone();

        join_set.spawn(async move {
            let _permit    = permit.acquire_owned().await;
            let agent_id   = agent.agent.id.clone();
            let agent_name = agent.agent.name.clone();
            let agent_icon = agent.agent.icon.clone();

            // ── Check cache before running ─────────────────────────────
            let blob_shas  = ctx.blob_shas.clone();
            let cache_hits = {
                let c = cache.lock().await;
                c.hits_for_agent(&agent_id, &blob_shas)
            };

            // Files in the diff that this agent could review (filtered by agent
            // patterns but not yet by the cache)
            let diff_files: Vec<String> = ctx.blob_shas.keys().cloned().collect();
            let uncached: Vec<String> = diff_files
                .iter()
                .filter(|f| !cache_hits.contains(f))
                .cloned()
                .collect();

            debug!(
                agent_id = %agent_id,
                total = diff_files.len(),
                cached = cache_hits.len(),
                uncached = uncached.len(),
                "Cache check"
            );

            // ── Full cache hit — skip LLM call ─────────────────────────
            if uncached.is_empty() && !diff_files.is_empty() {
                let cached_comments = {
                    let c = cache.lock().await;
                    c.valid_comments_for_agent(&agent_id, &blob_shas)
                };
                let comment_count  = cached_comments.len();
                let reason = format!(
                    "{} file(s) from cache, 0 tokens used",
                    cache_hits.len()
                );
                info!(agent_id = %agent_id, comment_count, "Full cache hit — skipping LLM");

                let _ = tx.send(AgentUpdate {
                    agent_id: agent_id.clone(),
                    status: AgentStatus::Skipped { reason: reason.clone() },
                }).await;

                return Some(AgentFinding {
                    agent_id,
                    agent_name,
                    agent_icon,
                    comments: cached_comments,
                });
            }

            // ── Partial or full miss — run the agent ───────────────────
            // Build a context that skips already-cached files
            let run_ctx: Arc<ReviewContext> = if cache_hits.is_empty() {
                Arc::clone(&ctx)
            } else {
                let mut c = (*ctx).clone();
                c.cache_skip_paths = cache_hits.clone();
                Arc::new(c)
            };

            let _ = tx.send(AgentUpdate {
                agent_id: agent_id.clone(),
                status: AgentStatus::Running { started_at: chrono::Utc::now() },
            }).await;

            let status = runner.run(&agent, &run_ctx).await;

            let finding = match &status {
                AgentStatus::Done { comments, elapsed_ms, .. } => {
                    info!(
                        agent_id = %agent_id,
                        comment_count = comments.len(),
                        cached_files  = cache_hits.len(),
                        elapsed_ms,
                        "Agent done"
                    );

                    // Save new results to cache
                    {
                        let mut c = cache.lock().await;
                        c.put_agent_results(&agent_id, comments, &blob_shas);
                    }

                    // Merge new comments with previously-cached ones
                    let mut all_comments = comments.clone();
                    if !cache_hits.is_empty() {
                        let cached = {
                            let c = cache.lock().await;
                            c.valid_comments_for_agent(&agent_id, &blob_shas)
                        };
                        // Avoid double-counting the new comments we just saved
                        let new_paths: std::collections::HashSet<_> = comments
                            .iter()
                            .filter_map(|c| c.file_path.as_deref())
                            .collect();
                        let extra: Vec<_> = cached
                            .into_iter()
                            .filter(|c| {
                                c.file_path
                                    .as_deref()
                                    .map(|p| !new_paths.contains(p))
                                    .unwrap_or(true)
                            })
                            .collect();
                        all_comments.extend(extra);
                        warn!(
                            agent_id = %agent_id,
                            merged_from_cache = cache_hits.len(),
                            "Merged cached comments with new agent results"
                        );
                    }

                    Some(AgentFinding {
                        agent_id: agent_id.clone(),
                        agent_name,
                        agent_icon,
                        comments: all_comments,
                    })
                }
                AgentStatus::Failed { error } => {
                    error!(agent_id = %agent_id, error = %error, "Agent failed");
                    None
                }
                _ => None,
            };

            let _ = tx.send(AgentUpdate { agent_id, status }).await;
            finding
        });
    }

    let mut findings: Vec<AgentFinding> = Vec::new();
    while let Some(result) = join_set.join_next().await {
        if let Ok(Some(finding)) = result {
            findings.push(finding);
        }
    }
    findings.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
    findings
}

// ── Phase 2 (synthesis, no cache) ────────────────────────────────────────────

async fn run_phase(
    agents: Vec<AgentDefinition>,
    ctx: Arc<ReviewContext>,
    runner: Arc<AgentRunner>,
    concurrency: usize,
    tx: &mpsc::Sender<AgentUpdate>,
) -> Vec<AgentFinding> {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut join_set = JoinSet::new();

    for agent in agents {
        let permit     = Arc::clone(&semaphore);
        let runner     = Arc::clone(&runner);
        let ctx        = Arc::clone(&ctx);
        let tx         = tx.clone();

        join_set.spawn(async move {
            let _permit    = permit.acquire_owned().await;
            let agent_id   = agent.agent.id.clone();
            let agent_name = agent.agent.name.clone();
            let agent_icon = agent.agent.icon.clone();

            debug!(agent_id = %agent_id, "Agent starting");

            let _ = tx.send(AgentUpdate {
                agent_id: agent_id.clone(),
                status: AgentStatus::Running { started_at: chrono::Utc::now() },
            }).await;

            let status = runner.run(&agent, &ctx).await;

            let finding = match &status {
                AgentStatus::Done { comments, elapsed_ms, .. } => {
                    info!(agent_id = %agent_id, comment_count = comments.len(),
                          elapsed_ms, "Agent done");
                    Some(AgentFinding {
                        agent_id: agent_id.clone(),
                        agent_name,
                        agent_icon,
                        comments: comments.clone(),
                    })
                }
                AgentStatus::Failed { error } => {
                    error!(agent_id = %agent_id, error = %error, "Agent failed");
                    None
                }
                _ => None,
            };

            let _ = tx.send(AgentUpdate { agent_id, status }).await;
            finding
        });
    }

    let mut findings: Vec<AgentFinding> = Vec::new();
    while let Some(result) = join_set.join_next().await {
        if let Ok(Some(finding)) = result {
            findings.push(finding);
        }
    }
    findings.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
    findings
}
