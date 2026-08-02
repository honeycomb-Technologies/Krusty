use std::collections::HashMap;
use std::sync::Mutex;

use mitsuro_hive_protocol::EventEnvelope;
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
        channels.retain(|_, sender| sender.receiver_count() > 0);
        channels
            .entry(session_id.to_string())
            .or_insert_with(|| broadcast::channel(self.capacity).0)
            .subscribe()
    }

    pub(crate) fn publish(&self, event: EventEnvelope) {
        let Some(session_id) = event.session_id.clone() else {
            return;
        };
        let sender = {
            let mut channels = self
                .channels
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let sender = channels
                .get(&session_id)
                .filter(|sender| sender.receiver_count() > 0)
                .cloned();
            if sender.is_none() {
                channels.remove(&session_id);
            }
            sender
        };
        let Some(sender) = sender else {
            return;
        };
        if sender.send(event).is_err() {
            let mut channels = self
                .channels
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if channels
                .get(&session_id)
                .is_some_and(|current| current.receiver_count() == 0)
            {
                channels.remove(&session_id);
            }
        }
    }

    #[cfg(test)]
    fn channel_count(&self) -> usize {
        self.channels
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    }
}

#[cfg(test)]
mod tests {
    use mitsuro_hive_protocol::{EventEnvelope, HiveEvent, ProtocolVersion, RuntimeEvent};

    use super::EventHub;

    fn event(session_id: &str, sequence: i64) -> EventEnvelope {
        EventEnvelope {
            version: ProtocolVersion::CURRENT,
            session_id: Some(session_id.to_string()),
            run_id: None,
            sequence: Some(sequence),
            emitted_at_unix_ms: 0,
            event: HiveEvent::Runtime(RuntimeEvent {
                event_type: "test".into(),
                payload: serde_json::json!({}),
            }),
        }
    }

    #[tokio::test]
    async fn session_churn_does_not_retain_channels_without_live_subscribers() {
        let hub = EventHub::new(8);
        for index in 0..5_000 {
            let session_id = format!("session-{index}");
            hub.publish(event(&session_id, 1));
            let receiver = hub.subscribe(&session_id);
            drop(receiver);
        }
        // The last dropped subscriber may remain until the next operation;
        // any new subscription prunes every zero-receiver entry.
        let mut active = hub.subscribe("active");
        assert_eq!(hub.channel_count(), 1);

        hub.publish(event("active", 7));
        assert_eq!(active.recv().await.unwrap().sequence, Some(7));
        hub.publish(event("no-subscriber", 1));
        assert_eq!(hub.channel_count(), 1);
    }
}
