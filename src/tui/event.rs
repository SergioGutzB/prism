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
    // Note: Tick is NOT sent through this channel.
    // Rendering is driven by a fixed tokio::time::interval in main.
    AgentUpdate(AgentUpdate),
    PrListLoaded(Vec<PrSummary>),
    PrLoaded(Box<PrDetails>),
    DiffLoaded(String),
    TicketLoaded(Option<Ticket>),
    Error(String),
    PublishDone,
    PublishFailed(String),
    /// Emitted when setup wizard saves config successfully.
    /// Carries (token, owner, repo) so main can reload and start.
    SetupSaved(String, String, String),
    SetupFailed(String),
    UserLoaded(String),
    ReviewsLoaded(Vec<crate::github::models::GhReview>, Vec<crate::github::models::GhPrComment>),
    QuickCommentDone,
    QuickCommentFailed(String),
    /// A streaming text chunk from Claude for fix-task at `index`.
    FixTaskChunk(usize, String),
    /// The fix-task at `index` completed successfully.
    FixTaskDone(usize),
    /// The fix-task at `index` failed with the given error message.
    FixTaskFailed(usize, String),
}

/// Spawn a background thread that polls crossterm key events.
/// Only key events are sent — rendering is driven by a separate tokio interval.
pub fn spawn_event_reader(tx: mpsc::UnboundedSender<AppEvent>) {
    std::thread::spawn(move || {
        // Poll with a short timeout so we never block the thread indefinitely.
        let poll_timeout = Duration::from_millis(5);
        loop {
            match event::poll(poll_timeout) {
                Ok(true) => match event::read() {
                    Ok(Event::Key(key)) => {
                        if tx.send(AppEvent::Key(key)).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {} // mouse, resize, paste — ignore
                    Err(_) => break,
                },
                Ok(false) => {} // no event — loop and poll again
                Err(_) => break,
            }
        }
    });
}
