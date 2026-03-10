use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

const PRESENCE_STALE_AFTER_SECS: i64 = 15;
const PRESENCE_EXPIRE_AFTER_SECS: i64 = 60;

pub type SessionPresenceMap = HashMap<String, HashMap<String, SessionPresenceRecord>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceCapability {
    Observer,
    Controller,
}

#[derive(Debug, Clone)]
pub struct SessionPresenceRecord {
    pub client_id: String,
    pub surface: String,
    pub capability: PresenceCapability,
    pub user_id: Option<String>,
    pub last_seen_at: DateTime<Utc>,
    pub last_event_sequence: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionPresenceClient {
    pub client_id: String,
    pub surface: String,
    pub capability: PresenceCapability,
    pub user_id: Option<String>,
    pub last_seen_at: String,
    pub last_event_sequence: Option<i64>,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionPresenceSnapshot {
    pub session_id: String,
    pub active_viewers: usize,
    pub active_controllers: usize,
    pub stale_clients: usize,
    pub clients: Vec<SessionPresenceClient>,
}

pub fn upsert_presence(
    registry: &mut SessionPresenceMap,
    session_id: &str,
    record: SessionPresenceRecord,
) -> SessionPresenceSnapshot {
    cleanup_presence_registry(registry);
    registry
        .entry(session_id.to_string())
        .or_default()
        .insert(record.client_id.clone(), record);
    snapshot_presence(registry, session_id)
}

pub fn remove_presence(
    registry: &mut SessionPresenceMap,
    session_id: &str,
    client_id: &str,
) -> SessionPresenceSnapshot {
    cleanup_presence_registry(registry);
    if let Some(session_presence) = registry.get_mut(session_id) {
        session_presence.remove(client_id);
        if session_presence.is_empty() {
            registry.remove(session_id);
        }
    }
    snapshot_presence(registry, session_id)
}

pub fn snapshot_presence(
    registry: &mut SessionPresenceMap,
    session_id: &str,
) -> SessionPresenceSnapshot {
    cleanup_presence_registry(registry);

    let now = Utc::now();
    let mut clients = registry
        .get(session_id)
        .map(|session_presence| {
            session_presence
                .values()
                .cloned()
                .map(|record| {
                    let stale = now.signed_duration_since(record.last_seen_at) > stale_threshold();
                    SessionPresenceClient {
                        client_id: record.client_id,
                        surface: record.surface,
                        capability: record.capability,
                        user_id: record.user_id,
                        last_seen_at: record.last_seen_at.to_rfc3339(),
                        last_event_sequence: record.last_event_sequence,
                        stale,
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    clients.sort_by(|left, right| left.client_id.cmp(&right.client_id));

    let active_viewers = clients.iter().filter(|client| !client.stale).count();
    let active_controllers = clients
        .iter()
        .filter(|client| !client.stale && client.capability == PresenceCapability::Controller)
        .count();
    let stale_clients = clients.iter().filter(|client| client.stale).count();

    SessionPresenceSnapshot {
        session_id: session_id.to_string(),
        active_viewers,
        active_controllers,
        stale_clients,
        clients,
    }
}

fn cleanup_presence_registry(registry: &mut SessionPresenceMap) {
    let now = Utc::now();
    registry.retain(|_, clients| {
        clients.retain(|_, record| {
            now.signed_duration_since(record.last_seen_at) <= expire_threshold()
        });
        !clients.is_empty()
    });
}

fn stale_threshold() -> Duration {
    Duration::seconds(PRESENCE_STALE_AFTER_SECS)
}

fn expire_threshold() -> Duration {
    Duration::seconds(PRESENCE_EXPIRE_AFTER_SECS)
}

#[cfg(test)]
mod tests {
    use super::{
        remove_presence, snapshot_presence, upsert_presence, PresenceCapability,
        SessionPresenceMap, SessionPresenceRecord,
    };
    use chrono::{Duration, Utc};

    #[test]
    fn snapshot_marks_stale_clients_without_dropping_them_immediately() {
        let mut registry = SessionPresenceMap::new();
        let session_id = "session-1";

        upsert_presence(
            &mut registry,
            session_id,
            SessionPresenceRecord {
                client_id: "client-1".to_string(),
                surface: "pwa".to_string(),
                capability: PresenceCapability::Controller,
                user_id: None,
                last_seen_at: Utc::now() - Duration::seconds(20),
                last_event_sequence: Some(12),
            },
        );

        let snapshot = snapshot_presence(&mut registry, session_id);
        assert_eq!(snapshot.active_viewers, 0);
        assert_eq!(snapshot.stale_clients, 1);
        assert!(snapshot.clients[0].stale);
    }

    #[test]
    fn remove_presence_clears_client_from_snapshot() {
        let mut registry = SessionPresenceMap::new();
        let session_id = "session-2";

        upsert_presence(
            &mut registry,
            session_id,
            SessionPresenceRecord {
                client_id: "client-1".to_string(),
                surface: "pwa".to_string(),
                capability: PresenceCapability::Observer,
                user_id: Some("alice".to_string()),
                last_seen_at: Utc::now(),
                last_event_sequence: None,
            },
        );

        let snapshot = remove_presence(&mut registry, session_id, "client-1");
        assert_eq!(snapshot.active_viewers, 0);
        assert!(snapshot.clients.is_empty());
    }
}
