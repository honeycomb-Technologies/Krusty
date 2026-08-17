//! Multi-provider connection identity and lifecycle registry.
//!
//! The desktop presentation still migrates incrementally, but provider
//! ownership starts here: a connection has a stable identity, its own backend,
//! status, and generation. Adding or reconnecting one entry never replaces an
//! unrelated provider.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Arc;
use std::time::SystemTime;

use mitsuro_desktop_backend::{
    BackendCapabilities, BackendKind, BackendProvenance, CapabilityNegotiation, DesktopBackend,
    LifecycleNotification, SessionSummary,
};

const MAX_BUFFERED_LIFECYCLE_EVENTS: usize = 256;

/// Stable identity for one configured backend connection.
///
/// `primary` is the default local instance. `named` leaves room for multiple
/// Mitsuro servers or remote Codex hosts without treating provider-native
/// session ids as globally unique.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConnectionId(String);

impl ConnectionId {
    pub fn primary(kind: BackendKind) -> Self {
        Self(kind.id().to_owned())
    }

    pub fn named(kind: BackendKind, name: &str) -> Result<Self, &'static str> {
        let name = name.trim();
        if name.is_empty() {
            return Err("connection name cannot be empty");
        }
        if name.contains(':') {
            return Err("connection name cannot contain ':'");
        }
        Ok(Self(format!("{}:{name}", kind.id())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn kind(&self) -> BackendKind {
        let kind_id = self
            .0
            .split_once(':')
            .map_or(self.0.as_str(), |(kind, _)| kind);
        BackendKind::from_id(kind_id).expect("ConnectionId constructors always validate backend")
    }

    pub fn name(&self) -> Option<&str> {
        self.0.split_once(':').map(|(_, name)| name)
    }

    pub fn parse_persisted(value: &str) -> Result<Self, &'static str> {
        let (kind_id, name) = value
            .split_once(':')
            .map_or((value, None), |(kind, name)| (kind, Some(name)));
        let kind = BackendKind::from_id(kind_id).ok_or("unknown connection backend")?;
        match name {
            Some(name) => Self::named(kind, name),
            None => Ok(Self::primary(kind)),
        }
    }
}

impl fmt::Display for ConnectionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Provider session identity qualified by its owning connection.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionKey {
    pub connection_id: ConnectionId,
    pub provider_session_id: String,
}

impl SessionKey {
    pub fn new(
        connection_id: ConnectionId,
        provider_session_id: impl Into<String>,
    ) -> Result<Self, &'static str> {
        let provider_session_id = provider_session_id.into();
        if provider_session_id.trim().is_empty() {
            return Err("provider session id cannot be empty");
        }
        Ok(Self {
            connection_id,
            provider_session_id,
        })
    }

    pub fn qualified(&self) -> String {
        format!("{}:{}", self.connection_id, self.provider_session_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connecting,
    Reconnecting,
    Ready {
        detail: String,
        has_auth: bool,
        session_count: usize,
    },
    Degraded {
        detail: String,
        has_auth: bool,
        session_count: usize,
        message: String,
    },
    Error {
        message: String,
    },
    Disconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconnectPolicy {
    Manual,
}

#[derive(Clone, Debug)]
pub struct ConnectionErrorRecord {
    pub message: String,
    pub occurred_at: SystemTime,
}

#[derive(Clone, Debug)]
pub struct ConnectionMetadata {
    pub display_name: String,
    pub provenance: BackendProvenance,
    pub capabilities: BackendCapabilities,
    pub capability_negotiation: CapabilityNegotiation,
    pub reconnect_policy: ReconnectPolicy,
    pub created_at: SystemTime,
    pub last_transition_at: SystemTime,
    pub ready_at: Option<SystemTime>,
    pub last_error: Option<ConnectionErrorRecord>,
}

impl ConnectionStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. } | Self::Degraded { .. })
    }
}

pub struct ConnectionEntry {
    pub id: ConnectionId,
    pub kind: BackendKind,
    pub backend: Arc<DesktopBackend>,
    pub status: ConnectionStatus,
    pub metadata: ConnectionMetadata,
    /// Last authoritative provider catalog. The summaries retain their native
    /// backend-qualified ids and remain available while another connection is
    /// selected.
    pub sessions: Vec<SessionSummary>,
    /// Provider-owned events received while another connection is selected.
    /// The bounded queue is replayed only into this connection's projection.
    pending_lifecycle_events: VecDeque<LifecycleNotification>,
    /// A bootstrap or event result may mutate this entry only when it carries
    /// the same generation.
    pub generation: u64,
}

impl fmt::Debug for ConnectionEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectionEntry")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("status", &self.status)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
pub struct ConnectionRegistry {
    entries: HashMap<ConnectionId, ConnectionEntry>,
    selected: Option<ConnectionId>,
    next_generation: u64,
}

impl ConnectionRegistry {
    /// Insert or reconnect one connection and return its new generation.
    /// Other entries are deliberately retained.
    pub fn begin_connect(&mut self, id: ConnectionId, backend: Arc<DesktopBackend>) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1);
        let generation = self.next_generation;
        let kind = backend.kind();
        let now = SystemTime::now();
        let previous = self.entries.remove(&id);
        let (sessions, pending_lifecycle_events, created_at, ready_at, last_error, display_name) =
            previous
                .map(|entry| {
                    (
                        entry.sessions,
                        entry.pending_lifecycle_events,
                        entry.metadata.created_at,
                        entry.metadata.ready_at,
                        entry.metadata.last_error,
                        entry.metadata.display_name,
                    )
                })
                .unwrap_or_else(|| {
                    (
                        Vec::new(),
                        VecDeque::new(),
                        now,
                        None,
                        None,
                        default_display_name(kind).to_owned(),
                    )
                });
        let metadata = ConnectionMetadata {
            display_name,
            provenance: backend.provenance(),
            capabilities: backend.capabilities(),
            capability_negotiation: backend.capability_negotiation(),
            reconnect_policy: ReconnectPolicy::Manual,
            created_at,
            last_transition_at: now,
            ready_at,
            last_error,
        };
        self.entries.insert(
            id.clone(),
            ConnectionEntry {
                id: id.clone(),
                kind,
                backend,
                status: ConnectionStatus::Connecting,
                metadata,
                sessions,
                pending_lifecycle_events,
                generation,
            },
        );
        self.selected.get_or_insert(id);
        generation
    }

    pub fn select(&mut self, id: &ConnectionId) -> bool {
        if !self.entries.contains_key(id) {
            return false;
        }
        self.selected = Some(id.clone());
        true
    }

    pub fn set_display_name(&mut self, id: &ConnectionId, display_name: String) -> bool {
        let Some(entry) = self.entries.get_mut(id) else {
            return false;
        };
        if display_name.trim().is_empty() {
            return false;
        }
        entry.metadata.display_name = display_name;
        true
    }

    pub fn selected_id(&self) -> Option<&ConnectionId> {
        self.selected.as_ref()
    }

    pub fn selected(&self) -> Option<&ConnectionEntry> {
        self.selected.as_ref().and_then(|id| self.entries.get(id))
    }

    pub fn get(&self, id: &ConnectionId) -> Option<&ConnectionEntry> {
        self.entries.get(id)
    }

    pub fn backend(&self, id: &ConnectionId) -> Option<Arc<DesktopBackend>> {
        self.get(id).map(|entry| Arc::clone(&entry.backend))
    }

    pub fn sessions(&self, id: &ConnectionId) -> Option<&[SessionSummary]> {
        self.get(id).map(|entry| entry.sessions.as_slice())
    }

    pub fn generation_matches(&self, id: &ConnectionId, generation: u64) -> bool {
        self.get(id)
            .is_some_and(|entry| entry.generation == generation)
    }

    /// Return the stable listener generation only for an already-ready
    /// connection. Provider selection can reuse this generation without
    /// treating navigation as a reconnect.
    pub fn ready_generation(&self, id: &ConnectionId) -> Option<u64> {
        self.get(id)
            .filter(|entry| entry.status.is_ready())
            .map(|entry| entry.generation)
    }

    pub fn queue_lifecycle_event(
        &mut self,
        id: &ConnectionId,
        generation: u64,
        event: LifecycleNotification,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(id) else {
            return false;
        };
        if entry.generation != generation {
            return false;
        }
        if entry.pending_lifecycle_events.len() == MAX_BUFFERED_LIFECYCLE_EVENTS {
            entry.pending_lifecycle_events.pop_front();
        }
        entry.pending_lifecycle_events.push_back(event);
        true
    }

    pub fn take_lifecycle_events(&mut self, id: &ConnectionId) -> Vec<LifecycleNotification> {
        self.entries
            .get_mut(id)
            .map(|entry| entry.pending_lifecycle_events.drain(..).collect())
            .unwrap_or_default()
    }

    pub fn pending_lifecycle_event_count(&self, id: &ConnectionId) -> usize {
        self.get(id)
            .map_or(0, |entry| entry.pending_lifecycle_events.len())
    }

    pub fn entries(&self) -> impl Iterator<Item = &ConnectionEntry> {
        self.entries.values()
    }

    pub fn backend_handles(&self) -> Vec<Arc<DesktopBackend>> {
        self.entries
            .values()
            .map(|entry| Arc::clone(&entry.backend))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn mark_ready(
        &mut self,
        id: &ConnectionId,
        generation: u64,
        detail: String,
        has_auth: bool,
        sessions: Vec<SessionSummary>,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(id) else {
            return false;
        };
        if entry.generation != generation {
            return false;
        }
        let session_count = sessions.len();
        entry.sessions = sessions;
        entry.status = ConnectionStatus::Ready {
            detail,
            has_auth,
            session_count,
        };
        let now = SystemTime::now();
        entry.metadata.last_transition_at = now;
        entry.metadata.ready_at = Some(now);
        entry.metadata.capability_negotiation = entry.backend.capability_negotiation();
        entry.metadata.capabilities = entry.metadata.capability_negotiation.advertised;
        true
    }

    pub fn mark_error(&mut self, id: &ConnectionId, generation: u64, message: String) -> bool {
        self.update_status(id, generation, ConnectionStatus::Error { message })
    }

    /// Retain a usable initialized transport and its last authoritative
    /// catalog when a non-destructive refresh fails.
    pub fn mark_degraded(&mut self, id: &ConnectionId, generation: u64, message: String) -> bool {
        let Some(entry) = self.entries.get_mut(id) else {
            return false;
        };
        if entry.generation != generation {
            return false;
        }
        let (detail, has_auth) = match &entry.status {
            ConnectionStatus::Ready {
                detail, has_auth, ..
            }
            | ConnectionStatus::Degraded {
                detail, has_auth, ..
            } => (detail.clone(), *has_auth),
            _ => return false,
        };
        let now = SystemTime::now();
        entry.metadata.last_transition_at = now;
        entry.metadata.last_error = Some(ConnectionErrorRecord {
            message: message.clone(),
            occurred_at: now,
        });
        entry.status = ConnectionStatus::Degraded {
            detail,
            has_auth,
            session_count: entry.sessions.len(),
            message,
        };
        true
    }

    pub fn mark_reconnecting(&mut self, id: &ConnectionId, generation: u64) -> bool {
        self.update_status(id, generation, ConnectionStatus::Reconnecting)
    }

    pub fn mark_disconnected(&mut self, id: &ConnectionId, generation: u64) -> bool {
        self.update_status(id, generation, ConnectionStatus::Disconnected)
    }

    fn update_status(
        &mut self,
        id: &ConnectionId,
        generation: u64,
        status: ConnectionStatus,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(id) else {
            return false;
        };
        if entry.generation != generation {
            return false;
        }
        let now = SystemTime::now();
        entry.metadata.last_transition_at = now;
        if let ConnectionStatus::Error { message } = &status {
            entry.metadata.last_error = Some(ConnectionErrorRecord {
                message: message.clone(),
                occurred_at: now,
            });
        }
        entry.status = status;
        true
    }
}

fn default_display_name(kind: BackendKind) -> &'static str {
    match kind {
        BackendKind::CodexStdio => "ChatGPT / Codex",
        BackendKind::MitsuroHttp => "Mitsuro server",
        BackendKind::CodexWebSocket => "Codex WebSocket",
        BackendKind::Fixture => "Offline fixtures",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend(kind: BackendKind) -> Arc<DesktopBackend> {
        match kind {
            BackendKind::CodexStdio => Arc::new(DesktopBackend::codex_stdio()),
            // Construction reads configuration only; no network call occurs.
            BackendKind::MitsuroHttp => {
                Arc::new(DesktopBackend::mitsuro_from_env().expect("default Mitsuro configuration"))
            }
            BackendKind::CodexWebSocket | BackendKind::Fixture => {
                panic!("test backend kind is unsupported")
            }
        }
    }

    #[test]
    fn adding_mitsuro_does_not_replace_codex() {
        let mut registry = ConnectionRegistry::default();
        let codex = ConnectionId::primary(BackendKind::CodexStdio);
        let mitsuro = ConnectionId::primary(BackendKind::MitsuroHttp);

        registry.begin_connect(codex.clone(), backend(BackendKind::CodexStdio));
        registry.begin_connect(mitsuro.clone(), backend(BackendKind::MitsuroHttp));

        assert_eq!(registry.len(), 2);
        assert_eq!(
            registry.get(&codex).map(|entry| entry.kind),
            Some(BackendKind::CodexStdio)
        );
        assert_eq!(
            registry.get(&mitsuro).map(|entry| entry.kind),
            Some(BackendKind::MitsuroHttp)
        );
    }

    #[test]
    fn stale_bootstrap_cannot_overwrite_reconnected_entry() {
        let mut registry = ConnectionRegistry::default();
        let id = ConnectionId::primary(BackendKind::CodexStdio);
        let stale = registry.begin_connect(id.clone(), backend(BackendKind::CodexStdio));
        let current = registry.begin_connect(id.clone(), backend(BackendKind::CodexStdio));

        assert!(!registry.mark_error(&id, stale, "stale failure".into()));
        assert!(registry.mark_ready(&id, current, "linux".into(), true, Vec::new()));
        assert!(registry
            .get(&id)
            .is_some_and(|entry| entry.status.is_ready()));
    }

    #[test]
    fn one_connection_failure_does_not_change_another_ready_provider() {
        let mut registry = ConnectionRegistry::default();
        let codex = ConnectionId::primary(BackendKind::CodexStdio);
        let mitsuro = ConnectionId::primary(BackendKind::MitsuroHttp);
        let codex_generation =
            registry.begin_connect(codex.clone(), backend(BackendKind::CodexStdio));
        let mitsuro_generation =
            registry.begin_connect(mitsuro.clone(), backend(BackendKind::MitsuroHttp));
        assert!(registry.mark_ready(&codex, codex_generation, "codex".into(), true, Vec::new()));
        assert!(registry.mark_error(&mitsuro, mitsuro_generation, "offline".into()));

        assert!(registry
            .get(&codex)
            .is_some_and(|entry| entry.status.is_ready()));
        assert!(matches!(
            registry.get(&mitsuro).map(|entry| &entry.status),
            Some(ConnectionStatus::Error { message }) if message == "offline"
        ));
        assert_eq!(registry.ready_generation(&codex), Some(codex_generation));
    }

    #[test]
    fn degraded_refresh_retains_transport_generation_and_authoritative_catalog() {
        let mut registry = ConnectionRegistry::default();
        let id = ConnectionId::primary(BackendKind::CodexStdio);
        let generation = registry.begin_connect(id.clone(), backend(BackendKind::CodexStdio));
        let session = SessionSummary {
            id: mitsuro_desktop_backend::BackendSessionId::new(BackendKind::CodexStdio, "thread-1"),
            title: Some("Retained".into()),
            preview: None,
            working_dir: None,
            updated_at: None,
            model_provider: None,
            ephemeral: false,
            archived: false,
        };
        assert!(registry.mark_ready(&id, generation, "linux".into(), true, vec![session.clone()],));
        assert!(registry.mark_degraded(&id, generation, "refresh failed".into()));

        assert_eq!(registry.ready_generation(&id), Some(generation));
        assert_eq!(registry.sessions(&id), Some([session].as_slice()));
        assert!(matches!(
            registry.get(&id).map(|entry| &entry.status),
            Some(ConnectionStatus::Degraded { message, .. }) if message == "refresh failed"
        ));
        assert!(registry.mark_reconnecting(&id, generation));
        assert_eq!(registry.ready_generation(&id), None);
    }

    #[test]
    fn connection_metadata_retains_provenance_capabilities_and_last_error() {
        let mut registry = ConnectionRegistry::default();
        let id = ConnectionId::primary(BackendKind::CodexStdio);
        let first = registry.begin_connect(id.clone(), backend(BackendKind::CodexStdio));
        let created_at = registry.get(&id).unwrap().metadata.created_at;
        let metadata = &registry.get(&id).unwrap().metadata;
        assert_eq!(metadata.display_name, "ChatGPT / Codex");
        assert!(metadata.capabilities.lifecycle_events);
        assert_eq!(
            metadata.capability_negotiation.schema_version,
            mitsuro_desktop_backend::DESKTOP_CAPABILITY_SCHEMA_VERSION
        );
        assert!(matches!(
            &metadata.provenance,
            BackendProvenance::SpawnedProcess { command } if command.contains("app-server")
        ));

        assert!(registry.mark_error(&id, first, "transport closed".into()));
        let failed_at = registry
            .get(&id)
            .and_then(|entry| entry.metadata.last_error.as_ref())
            .map(|error| error.occurred_at)
            .expect("last error timestamp");
        registry.begin_connect(id.clone(), backend(BackendKind::CodexStdio));
        let metadata = &registry.get(&id).unwrap().metadata;
        assert_eq!(metadata.created_at, created_at);
        assert_eq!(
            metadata
                .last_error
                .as_ref()
                .map(|error| error.message.as_str()),
            Some("transport closed")
        );
        assert_eq!(
            metadata.last_error.as_ref().map(|error| error.occurred_at),
            Some(failed_at)
        );
        assert_eq!(metadata.reconnect_policy, ReconnectPolicy::Manual);
    }

    #[test]
    fn reconnect_retains_the_last_authoritative_session_catalog() {
        let mut registry = ConnectionRegistry::default();
        let id = ConnectionId::primary(BackendKind::CodexStdio);
        let first = registry.begin_connect(id.clone(), backend(BackendKind::CodexStdio));
        let session = SessionSummary {
            id: mitsuro_desktop_backend::BackendSessionId::new(BackendKind::CodexStdio, "thread-1"),
            title: Some("Retained".into()),
            preview: None,
            working_dir: None,
            updated_at: None,
            model_provider: None,
            ephemeral: false,
            archived: false,
        };
        assert!(registry.mark_ready(&id, first, "linux".into(), true, vec![session.clone()],));

        registry.begin_connect(id.clone(), backend(BackendKind::CodexStdio));

        assert_eq!(registry.sessions(&id), Some([session].as_slice()));
    }

    #[test]
    fn selecting_a_ready_connection_reuses_its_listener_generation() {
        let mut registry = ConnectionRegistry::default();
        let codex = ConnectionId::primary(BackendKind::CodexStdio);
        let mitsuro = ConnectionId::primary(BackendKind::MitsuroHttp);
        let codex_generation =
            registry.begin_connect(codex.clone(), backend(BackendKind::CodexStdio));
        let mitsuro_generation =
            registry.begin_connect(mitsuro.clone(), backend(BackendKind::MitsuroHttp));
        assert!(registry.mark_ready(&codex, codex_generation, "codex".into(), true, Vec::new()));
        assert!(registry.mark_ready(
            &mitsuro,
            mitsuro_generation,
            "mitsuro".into(),
            true,
            Vec::new(),
        ));

        assert!(registry.select(&codex));
        assert_eq!(registry.ready_generation(&codex), Some(codex_generation));
        assert!(registry.select(&mitsuro));
        assert_eq!(
            registry.ready_generation(&mitsuro),
            Some(mitsuro_generation)
        );
        assert_eq!(registry.ready_generation(&codex), Some(codex_generation));
    }

    #[test]
    fn identical_provider_session_ids_remain_distinct() {
        let codex = SessionKey::new(ConnectionId::primary(BackendKind::CodexStdio), "same-id")
            .expect("Codex session key");
        let mitsuro = SessionKey::new(ConnectionId::primary(BackendKind::MitsuroHttp), "same-id")
            .expect("Mitsuro session key");

        assert_ne!(codex, mitsuro);
        assert_ne!(codex.qualified(), mitsuro.qualified());
    }

    #[test]
    fn named_connections_are_validated() {
        assert!(ConnectionId::named(BackendKind::MitsuroHttp, "local").is_ok());
        assert!(ConnectionId::named(BackendKind::MitsuroHttp, "").is_err());
        assert!(ConnectionId::named(BackendKind::MitsuroHttp, "bad:name").is_err());
        assert_eq!(
            ConnectionId::parse_persisted("mitsuro-http:local")
                .expect("persisted named connection")
                .as_str(),
            "mitsuro-http:local"
        );
        assert!(ConnectionId::parse_persisted("unknown:local").is_err());
    }

    #[test]
    fn inactive_connection_events_are_bounded_and_provider_owned() {
        let mut registry = ConnectionRegistry::default();
        let codex = ConnectionId::primary(BackendKind::CodexStdio);
        let mitsuro = ConnectionId::primary(BackendKind::MitsuroHttp);
        let codex_generation =
            registry.begin_connect(codex.clone(), backend(BackendKind::CodexStdio));
        registry.begin_connect(mitsuro.clone(), backend(BackendKind::MitsuroHttp));
        let event = LifecycleNotification::from_known("thread/started", None)
            .expect("known lifecycle event");

        for _ in 0..(MAX_BUFFERED_LIFECYCLE_EVENTS + 8) {
            assert!(registry.queue_lifecycle_event(&codex, codex_generation, event.clone(),));
        }

        assert_eq!(
            registry.pending_lifecycle_event_count(&codex),
            MAX_BUFFERED_LIFECYCLE_EVENTS
        );
        assert_eq!(registry.pending_lifecycle_event_count(&mitsuro), 0);
        assert_eq!(
            registry.take_lifecycle_events(&codex).len(),
            MAX_BUFFERED_LIFECYCLE_EVENTS
        );
        assert_eq!(registry.pending_lifecycle_event_count(&codex), 0);
    }
}
