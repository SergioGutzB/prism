pub mod extractor;
pub mod models;
pub mod provider;
pub mod providers;

use anyhow::Result;
use tracing::warn;

use crate::config::AppConfig;
use crate::tickets::provider::TicketProvider;
use crate::tickets::providers::jira::JiraProvider;

/// Build the list of configured ticket providers from app config.
pub fn build_providers(config: &AppConfig) -> Vec<Box<dyn TicketProvider>> {
    let mut providers: Vec<Box<dyn TicketProvider>> = Vec::new();

    for pc in &config.tickets.providers {
        if !pc.enabled {
            continue;
        }
        match pc.provider_type.as_str() {
            "jira" => {
                let base_url = std::env::var("JIRA_BASE_URL")
                    .unwrap_or_else(|_| pc.base_url.clone());
                let email = std::env::var("JIRA_EMAIL")
                    .unwrap_or_else(|_| pc.email.clone());
                let api_token = std::env::var("JIRA_API_TOKEN")
                    .unwrap_or_else(|_| pc.api_token.clone());

                match JiraProvider::new(&base_url, &email, &api_token, pc.key_patterns.clone()) {
                    Ok(p) => providers.push(Box::new(p)),
                    Err(e) => warn!("Failed to create Jira provider: {}", e),
                }
            }
            unknown => {
                warn!("Unknown ticket provider type: {}", unknown);
            }
        }
    }

    providers
}
