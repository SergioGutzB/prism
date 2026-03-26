use std::time::Duration;

use regex::Regex;
use tracing::{debug, warn};

use crate::tickets::models::Ticket;
use crate::tickets::provider::TicketProvider;

/// Extract ticket keys from any text (PR title, branch name, commit message, etc.)
/// by testing against each provider's key patterns.
pub fn extract_ticket_keys(text: &str, providers: &[Box<dyn TicketProvider>]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();

    for provider in providers {
        for pattern in provider.key_patterns() {
            match Regex::new(pattern) {
                Ok(re) => {
                    for m in re.find_iter(text) {
                        let key = m.as_str().to_string();
                        if !keys.contains(&key) {
                            keys.push(key);
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        provider = provider.name(),
                        pattern = pattern,
                        "Invalid key pattern: {}",
                        e
                    );
                }
            }
        }
    }

    debug!("Extracted ticket keys: {:?}", keys);
    keys
}

/// Try each extracted key against each provider, returning the first ticket found.
///
/// Each provider call has a 5-second timeout so that a slow/unavailable
/// provider can never stall the UI.
pub async fn resolve_ticket(
    keys: &[String],
    providers: &[Box<dyn TicketProvider>],
) -> Option<Ticket> {
    let timeout = Duration::from_secs(5);

    for key in keys {
        for provider in providers {
            let fetch = provider.get_ticket(key);
            match tokio::time::timeout(timeout, fetch).await {
                Ok(Ok(Some(ticket))) => {
                    debug!(key = %key, provider = provider.name(), "Resolved ticket");
                    return Some(ticket);
                }
                Ok(Ok(None)) => {
                    debug!(key = %key, provider = provider.name(), "Ticket not found");
                }
                Ok(Err(e)) => {
                    warn!(
                        key = %key,
                        provider = provider.name(),
                        "Error fetching ticket: {}",
                        e
                    );
                }
                Err(_) => {
                    warn!(
                        key = %key,
                        provider = provider.name(),
                        "Ticket fetch timed out after 5s"
                    );
                }
            }
        }
    }

    None
}
