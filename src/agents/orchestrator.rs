use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tracing::{debug, error, info};

use crate::agents::context::ReviewContext;
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

    /// Run all enabled agents concurrently, respecting the concurrency limit.
    ///
    /// Status updates are sent through the returned `mpsc::Receiver`.
    /// The function returns immediately after spawning tasks.
    pub fn run_all(
        &self,
        agents: Vec<AgentDefinition>,
        ctx: ReviewContext,
    ) -> mpsc::Receiver<AgentUpdate> {
        let (tx, rx) = mpsc::channel::<AgentUpdate>(128);
        let runner = Arc::clone(&self.runner);
        let concurrency = self.concurrency;
        let ctx = Arc::new(ctx);

        tokio::spawn(async move {
            // Split into enabled / disabled
            let (enabled, disabled): (Vec<_>, Vec<_>) =
                agents.into_iter().partition(|a| a.agent.enabled);

            // Immediately emit Disabled status for disabled agents
            for agent in disabled {
                let _ = tx
                    .send(AgentUpdate {
                        agent_id: agent.agent.id.clone(),
                        status: AgentStatus::Disabled,
                    })
                    .await;
            }

            // Emit Pending for all enabled agents
            for agent in &enabled {
                let _ = tx
                    .send(AgentUpdate {
                        agent_id: agent.agent.id.clone(),
                        status: AgentStatus::Pending,
                    })
                    .await;
            }

            // Run with concurrency limit using a semaphore
            let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
            let mut join_set = JoinSet::new();

            for agent in enabled {
                let permit = Arc::clone(&semaphore);
                let runner = Arc::clone(&runner);
                let ctx = Arc::clone(&ctx);
                let tx = tx.clone();

                join_set.spawn(async move {
                    let _permit = permit.acquire_owned().await;

                    let agent_id = agent.agent.id.clone();
                    debug!(agent_id = %agent_id, "Agent starting");

                    // Emit Running
                    let _ = tx
                        .send(AgentUpdate {
                            agent_id: agent_id.clone(),
                            status: AgentStatus::Running {
                                started_at: chrono::Utc::now(),
                            },
                        })
                        .await;

                    let status = runner.run(&agent, &ctx).await;

                    match &status {
                        AgentStatus::Done { comments, elapsed_ms } => {
                            info!(
                                agent_id = %agent_id,
                                comment_count = comments.len(),
                                elapsed_ms = elapsed_ms,
                                "Agent done"
                            );
                        }
                        AgentStatus::Failed { error } => {
                            error!(agent_id = %agent_id, error = %error, "Agent failed");
                        }
                        _ => {}
                    }

                    let _ = tx.send(AgentUpdate { agent_id, status }).await;
                });
            }

            // Wait for all tasks to complete
            while join_set.join_next().await.is_some() {}

            info!("All agents completed");
        });

        rx
    }
}
