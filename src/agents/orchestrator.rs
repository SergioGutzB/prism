use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tracing::{debug, error, info};

use crate::agents::context::{AgentFinding, ObjectiveAnalysis, ReviewContext};
use crate::agents::models::{AgentDefinition, AgentStatus};
use crate::agents::runner::AgentRunner;
use crate::config::AppConfig;

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

    /// Run all enabled agents in three collaborative phases.
    ///
    /// **Phase 0** — Objective-validator agents (`phase_zero = true`) run first,
    /// sequentially. Their `ObjectiveAnalysis` output is injected into all later
    /// agent contexts so every specialist knows whether the PR aligns with the
    /// ticket objectives.
    ///
    /// **Phase 1** — Specialist agents (`synthesis = false`, `phase_zero = false`)
    /// run concurrently with the enriched context.
    ///
    /// **Phase 2** — Synthesis agents (`synthesis = true`) run after Phase 1
    /// completes, receiving both the objective analysis and the specialist findings.
    ///
    /// Status updates are emitted through the returned `mpsc::Receiver`.
    /// The function returns immediately; the agent tasks run in the background.
    pub fn run_all(
        &self,
        agents: Vec<AgentDefinition>,
        ctx: ReviewContext,
    ) -> mpsc::Receiver<AgentUpdate> {
        let (tx, rx) = mpsc::channel::<AgentUpdate>(256);
        let runner = Arc::clone(&self.runner);
        let concurrency = self.concurrency;
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

            // Emit Pending for all enabled agents (all phases)
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

            // ── Phase 1: specialist agents ─────────────────────────────────
            let phase1_findings = run_phase(
                specialists,
                Arc::clone(&ctx1),
                Arc::clone(&runner),
                concurrency,
                &tx,
            ).await;

            // ── Phase 2: synthesis agents with enriched context ────────────
            if !synthesis.is_empty() {
                let mut ctx2 = (*ctx1).clone();
                ctx2.prior_findings = phase1_findings;
                let ctx2 = Arc::new(ctx2);

                run_phase(
                    synthesis,
                    ctx2,
                    Arc::clone(&runner),
                    concurrency,
                    &tx,
                ).await;
            }

            info!("All agents completed");
        });

        rx
    }
}

/// Run Phase-0 objective-validator agents sequentially.
///
/// There will typically be just one, but the design allows for multiple.
/// Returns the last successful `ObjectiveAnalysis`.
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
            info!(
                agent_id = %agent_id,
                comment_count = comments.len(),
                elapsed_ms = elapsed_ms,
                "Phase-0 agent done"
            );
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

/// Run a list of agents concurrently (up to `concurrency` at once).
///
/// Returns the aggregated specialist findings for Phase-2 context enrichment.
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
        let permit = Arc::clone(&semaphore);
        let runner = Arc::clone(&runner);
        let ctx = Arc::clone(&ctx);
        let tx = tx.clone();

        join_set.spawn(async move {
            let _permit = permit.acquire_owned().await;
            let agent_id = agent.agent.id.clone();
            let agent_name = agent.agent.name.clone();
            let agent_icon = agent.agent.icon.clone();

            debug!(agent_id = %agent_id, "Agent starting");

            let _ = tx.send(AgentUpdate {
                agent_id: agent_id.clone(),
                status: AgentStatus::Running { started_at: chrono::Utc::now() },
            }).await;

            let status = runner.run(&agent, &ctx).await;

            // Extract comments for Phase-2 findings aggregation
            let finding = match &status {
                AgentStatus::Done { comments, elapsed_ms, .. } => {
                    info!(
                        agent_id = %agent_id,
                        comment_count = comments.len(),
                        elapsed_ms = elapsed_ms,
                        "Agent done"
                    );
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

    // Collect results preserving agent order
    let mut findings: Vec<AgentFinding> = Vec::new();
    while let Some(result) = join_set.join_next().await {
        if let Ok(Some(finding)) = result {
            findings.push(finding);
        }
    }

    // Sort by agent_id so the synthesis prompt is deterministic
    findings.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
    findings
}
