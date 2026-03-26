use std::path::PathBuf;
use anyhow::Result;
use tracing::{debug, warn};

use crate::agents::models::AgentDefinition;
use crate::config::AppConfig;

// Built-in agents embedded at compile time
const BUILTIN_SECURITY: &str = include_str!("../../agents/security.toml");
const BUILTIN_ARCHITECTURE: &str = include_str!("../../agents/architecture.toml");
const BUILTIN_TESTS: &str = include_str!("../../agents/tests.toml");
const BUILTIN_PERFORMANCE: &str = include_str!("../../agents/performance.toml");
const BUILTIN_STYLE: &str = include_str!("../../agents/style.toml");
const BUILTIN_SUMMARY: &str = include_str!("../../agents/summary.toml");

const BUILTINS: &[&str] = &[
    BUILTIN_SECURITY,
    BUILTIN_ARCHITECTURE,
    BUILTIN_TESTS,
    BUILTIN_PERFORMANCE,
    BUILTIN_STYLE,
    BUILTIN_SUMMARY,
];

/// Load all agent definitions.
///
/// Built-in agents are always loaded first. User-defined agents (from
/// `~/.config/prism/agents/*.toml`) override built-ins with the same `id`.
/// The final list is sorted by `order`.
pub fn load_agents(config: &AppConfig) -> Result<Vec<AgentDefinition>> {
    let mut agents: Vec<AgentDefinition> = Vec::new();

    // Load built-ins
    for source in BUILTINS {
        match toml::from_str::<AgentDefinition>(source) {
            Ok(def) => {
                debug!("Loaded built-in agent: {}", def.agent.id);
                agents.push(def);
            }
            Err(e) => {
                warn!("Failed to parse built-in agent TOML: {}", e);
            }
        }
    }

    // Load user overrides
    let user_agents_dir = expand_tilde(&config.agents.agents_dir);
    if user_agents_dir.exists() {
        match std::fs::read_dir(&user_agents_dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                        match std::fs::read_to_string(&path) {
                            Ok(content) => match toml::from_str::<AgentDefinition>(&content) {
                                Ok(def) => {
                                    debug!(
                                        "Loaded user agent: {} from {}",
                                        def.agent.id,
                                        path.display()
                                    );
                                    // Replace built-in with same id if present
                                    if let Some(existing) =
                                        agents.iter_mut().find(|a| a.agent.id == def.agent.id)
                                    {
                                        *existing = def;
                                    } else {
                                        agents.push(def);
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to parse {}: {}", path.display(), e);
                                }
                            },
                            Err(e) => {
                                warn!("Failed to read {}: {}", path.display(), e);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!(
                    "Cannot read agents directory {}: {}",
                    user_agents_dir.display(),
                    e
                );
            }
        }
    }

    // Sort by order field
    agents.sort_by_key(|a| a.agent.order);

    Ok(agents)
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(stripped)
    } else {
        PathBuf::from(path)
    }
}
