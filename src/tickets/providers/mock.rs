use async_trait::async_trait;
use anyhow::Result;
use std::collections::HashMap;

use crate::tickets::models::Ticket;
use crate::tickets::provider::TicketProvider;

/// In-memory mock provider for testing.
pub struct MockProvider {
    name: String,
    key_patterns: Vec<String>,
    tickets: HashMap<String, Ticket>,
    available: bool,
}

impl MockProvider {
    pub fn new(name: &str, key_patterns: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            key_patterns,
            tickets: HashMap::new(),
            available: true,
        }
    }

    pub fn add_ticket(mut self, ticket: Ticket) -> Self {
        self.tickets.insert(ticket.key.clone(), ticket);
        self
    }

    pub fn set_available(mut self, available: bool) -> Self {
        self.available = available;
        self
    }
}

#[async_trait]
impl TicketProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn key_patterns(&self) -> &[String] {
        &self.key_patterns
    }

    async fn get_ticket(&self, key: &str) -> Result<Option<Ticket>> {
        Ok(self.tickets.get(key).cloned())
    }

    async fn is_available(&self) -> bool {
        self.available
    }
}
