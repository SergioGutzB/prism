use std::time::Duration;

use crossterm::event::{self, Event, KeyEvent};
use tokio::sync::mpsc;

use crate::agents::orchestrator::AgentUpdate;
use crate::github::models::{PrDetails, PrSummary};
use crate::tickets::models::Ticket;

/// All events that the main event loop processes.
#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    Tick,
    AgentUpdate(AgentUpdate),
    PrListLoaded(Vec<PrSummary>),
    PrLoaded(Box<PrDetails>),
    DiffLoaded(String),
    TicketLoaded(Option<Ticket>),
    Error(String),
    PublishDone,
    PublishFailed(String),
}

/// Spawn a background thread that polls crossterm events and sends them
/// through the channel at 16ms tick rate (~60fps).
pub fn spawn_event_reader(tx: mpsc::UnboundedSender<AppEvent>) {
    std::thread::spawn(move || {
        let tick = Duration::from_millis(16);
        loop {
            if event::poll(tick).unwrap_or(false) {
                match event::read() {
                    Ok(Event::Key(key)) => {
                        if tx.send(AppEvent::Key(key)).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {} // Mouse, resize, paste — ignore for now
                    Err(_) => break,
                }
            } else {
                // Tick
                if tx.send(AppEvent::Tick).is_err() {
                    break;
                }
            }
        }
    });
}
