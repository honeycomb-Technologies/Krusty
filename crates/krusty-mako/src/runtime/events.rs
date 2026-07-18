use std::collections::HashMap;
use std::sync::Mutex;

use krusty_mako_protocol::EventEnvelope;
use tokio::sync::broadcast;

pub(crate) struct EventHub {
    channels: Mutex<HashMap<String, broadcast::Sender<EventEnvelope>>>,
    capacity: usize,
}

impl EventHub {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            channels: Mutex::new(HashMap::new()),
            capacity,
        }
    }

    pub(crate) fn subscribe(&self, session_id: &str) -> broadcast::Receiver<EventEnvelope> {
        let mut channels = self
            .channels
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        channels
            .entry(session_id.to_string())
            .or_insert_with(|| broadcast::channel(self.capacity).0)
            .subscribe()
    }

    pub(crate) fn publish(&self, event: EventEnvelope) {
        let Some(session_id) = event.session_id.as_deref() else {
            return;
        };
        let sender = {
            let mut channels = self
                .channels
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            channels
                .entry(session_id.to_string())
                .or_insert_with(|| broadcast::channel(self.capacity).0)
                .clone()
        };
        let _ = sender.send(event);
    }
}
