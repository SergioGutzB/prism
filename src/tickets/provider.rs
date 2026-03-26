use async_trait::async_trait;
use anyhow::Result;
use crate::tickets::models::Ticket;

#[async_trait]
pub trait TicketProvider: Send + Sync {
    /// Display name for this provider.
    fn name(&self) -> &str;

    /// Regex patterns to match ticket keys (e.g. `["[A-Z]{2,10}-\\d+"]`).
    fn key_patterns(&self) -> &[String];

    /// Fetch a ticket by its key. Returns `None` if the ticket is not found.
    async fn get_ticket(&self, key: &str) -> Result<Option<Ticket>>;

    /// Check whether the provider is reachable (e.g. credentials are valid).
    async fn is_available(&self) -> bool;
}
