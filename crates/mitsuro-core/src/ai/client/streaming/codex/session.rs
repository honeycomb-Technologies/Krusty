use dashmap::DashMap;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const MAX_CODEX_WS_SESSIONS: usize = 32;
const CODEX_WS_IDLE_TTL: Duration = Duration::from_secs(5 * 60);
const CODEX_WS_REAPER_INTERVAL: Duration = Duration::from_secs(30);
const CODEX_WS_MAX_REUSE_AGE: Duration = Duration::from_secs(55 * 60);

pub(crate) type CodexWebSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug, Clone)]
pub(crate) struct CodexContinuation {
    pub response_id: String,
    pub request_fingerprint: String,
    pub message_fingerprints: Vec<String>,
    pub assistant_fingerprint: Option<String>,
    pub volatile_context_fingerprint: Option<String>,
}

#[derive(Default)]
pub(crate) struct CodexSessionState {
    pub connection: Option<CodexWebSocket>,
    pub connected_at: Option<Instant>,
    pub continuation: Option<CodexContinuation>,
}

impl CodexSessionState {
    pub fn can_reuse_connection(&self) -> bool {
        self.connection.is_some()
            && self
                .connected_at
                .is_some_and(|started| connection_age_is_reusable(started, Instant::now()))
    }

    pub fn reset(&mut self) {
        self.connection = None;
        self.connected_at = None;
        self.continuation = None;
    }
}

pub(crate) struct CodexWsSession {
    state: Arc<Mutex<CodexSessionState>>,
    last_used: AtomicU64,
}

impl CodexWsSession {
    fn new_at(now: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(CodexSessionState::default())),
            last_used: AtomicU64::new(now),
        }
    }

    fn touch_at(&self, now: u64) {
        self.last_used.store(now, Ordering::SeqCst);
    }

    pub async fn lock_owned(self: &Arc<Self>) -> CodexSessionGuard {
        self.lock_owned_at(now_epoch_seconds()).await
    }

    async fn lock_owned_at(self: &Arc<Self>, now: u64) -> CodexSessionGuard {
        self.touch_at(now);
        CodexSessionGuard {
            state: Arc::clone(&self.state).lock_owned().await,
            session: Some(Arc::clone(self)),
        }
    }
}

/// Owned state guard that records idleness from the end of a response, not its
/// start. Holding the session Arc also makes in-flight eviction impossible.
pub(crate) struct CodexSessionGuard {
    state: OwnedMutexGuard<CodexSessionState>,
    session: Option<Arc<CodexWsSession>>,
}

impl CodexSessionGuard {
    pub async fn ephemeral() -> Self {
        Self {
            state: Arc::new(Mutex::new(CodexSessionState::default()))
                .lock_owned()
                .await,
            session: None,
        }
    }
}

impl Deref for CodexSessionGuard {
    type Target = CodexSessionState;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for CodexSessionGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Drop for CodexSessionGuard {
    fn drop(&mut self) {
        if let Some(session) = &self.session {
            session.touch_at(now_epoch_seconds());
        }
    }
}

/// Bounded, session-keyed transport state for Codex Responses WebSockets.
///
/// A single socket supports one in-flight response, so each session owns a
/// mutex while different sessions remain independent. The hard cap prevents a
/// long-lived server from retaining an unbounded number of idle sockets.
pub(crate) struct CodexWsPool {
    sessions: Arc<DashMap<String, Arc<CodexWsSession>>>,
    creation_gate: Arc<Mutex<()>>,
    reaper_started: AtomicBool,
}

impl Default for CodexWsPool {
    fn default() -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            creation_gate: Arc::new(Mutex::new(())),
            reaper_started: AtomicBool::new(false),
        }
    }
}

impl CodexWsPool {
    pub async fn session(&self, key: &str) -> Arc<CodexWsSession> {
        self.ensure_reaper_started();
        loop {
            if let Some(session) = self.try_session_at(key, now_epoch_seconds()).await {
                return session;
            }

            // Every slot is actively borrowed or has an in-flight response.
            // Backpressure instead of creating a second socket for an evicted
            // session and violating the one-in-flight contract.
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[cfg(test)]
    async fn session_at(&self, key: &str, now: u64) -> Arc<CodexWsSession> {
        loop {
            if let Some(session) = self.try_session_at(key, now).await {
                return session;
            }
            tokio::task::yield_now().await;
        }
    }

    async fn try_session_at(&self, key: &str, now: u64) -> Option<Arc<CodexWsSession>> {
        let _gate = self.creation_gate.lock().await;
        self.evict_expired_idle_at(now);

        if let Some(existing) = self.sessions.get(key) {
            existing.touch_at(now);
            return Some(Arc::clone(existing.value()));
        }

        if self.sessions.len() < MAX_CODEX_WS_SESSIONS || self.evict_oldest_idle() {
            let session = Arc::new(CodexWsSession::new_at(now));
            self.sessions.insert(key.to_string(), Arc::clone(&session));
            return Some(session);
        }

        None
    }

    fn ensure_reaper_started(&self) -> bool {
        if self
            .reaper_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }

        let sessions = Arc::downgrade(&self.sessions);
        let creation_gate = Arc::downgrade(&self.creation_gate);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(CODEX_WS_REAPER_INTERVAL).await;
                let (Some(sessions), Some(creation_gate)) =
                    (sessions.upgrade(), creation_gate.upgrade())
                else {
                    break;
                };
                let _gate = creation_gate.lock().await;
                evict_expired_idle_sessions(&sessions, now_epoch_seconds());
            }
        });
        true
    }

    fn evict_expired_idle_at(&self, now: u64) {
        evict_expired_idle_sessions(&self.sessions, now);
    }

    fn evict_oldest_idle(&self) -> bool {
        let oldest = self
            .sessions
            .iter()
            .filter(|entry| is_idle_session(entry.value()))
            .map(|entry| {
                (
                    entry.key().clone(),
                    entry.value().last_used.load(Ordering::SeqCst),
                )
            })
            .min_by_key(|(_, last_used)| *last_used);

        oldest
            .and_then(|(key, observed_last_used)| {
                self.sessions.remove_if(&key, |_, session| {
                    is_idle_session(session)
                        && session.last_used.load(Ordering::SeqCst) == observed_last_used
                })
            })
            .is_some()
    }
}

fn evict_expired_idle_sessions(sessions: &DashMap<String, Arc<CodexWsSession>>, now: u64) {
    let expired = sessions
        .iter()
        .filter(|entry| {
            let session = entry.value();
            is_idle_session(session)
                && is_idle_expired(session.last_used.load(Ordering::SeqCst), now)
        })
        .map(|entry| {
            (
                entry.key().clone(),
                entry.value().last_used.load(Ordering::SeqCst),
            )
        })
        .collect::<Vec<_>>();

    for (key, observed_last_used) in expired {
        sessions.remove_if(&key, |_, session| {
            is_idle_session(session)
                && session.last_used.load(Ordering::SeqCst) == observed_last_used
                && is_idle_expired(observed_last_used, now)
        });
    }
}

fn is_idle_session(session: &Arc<CodexWsSession>) -> bool {
    Arc::strong_count(session) == 1 && Arc::strong_count(&session.state) == 1
}

fn is_idle_expired(last_used: u64, now: u64) -> bool {
    now.saturating_sub(last_used) >= CODEX_WS_IDLE_TTL.as_secs()
}

fn connection_age_is_reusable(connected_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(connected_at) < CODEX_WS_MAX_REUSE_AGE
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pool_is_bounded() {
        let pool = CodexWsPool::default();
        for index in 0..(MAX_CODEX_WS_SESSIONS + 5) {
            let _ = pool.session_at(&format!("session-{index}"), 0).await;
        }
        assert!(pool.sessions.len() <= MAX_CODEX_WS_SESSIONS);
    }

    #[tokio::test]
    async fn background_reaper_starts_once_without_retaining_the_pool() {
        let pool = CodexWsPool::default();
        let sessions = Arc::downgrade(&pool.sessions);

        assert!(pool.ensure_reaper_started());
        assert!(!pool.ensure_reaper_started());
        assert!(pool.reaper_started.load(Ordering::Acquire));

        drop(pool);
        assert!(sessions.upgrade().is_none());
    }

    #[tokio::test]
    async fn idle_expiry_uses_five_minutes_from_latest_access() {
        let pool = CodexWsPool::default();
        let original = pool.session_at("session", 100).await;

        let before_ttl = pool.session_at("session", 399).await;
        assert!(Arc::ptr_eq(&original, &before_ttl));
        let original_weak = Arc::downgrade(&original);
        drop(before_ttl);
        drop(original);

        let at_refreshed_ttl = pool.session_at("session", 699).await;
        assert!(original_weak.upgrade().is_none());
        assert_eq!(pool.sessions.len(), 1);
        assert_eq!(at_refreshed_ttl.last_used.load(Ordering::Relaxed), 699);
    }

    #[tokio::test]
    async fn expired_idle_sessions_are_swept_below_the_pool_cap() {
        let pool = CodexWsPool::default();
        let expired = pool.session_at("expired", 10).await;
        drop(expired);

        let _current = pool
            .session_at("current", 10 + CODEX_WS_IDLE_TTL.as_secs())
            .await;

        assert!(!pool.sessions.contains_key("expired"));
        assert!(pool.sessions.contains_key("current"));
        assert_eq!(pool.sessions.len(), 1);
    }

    #[tokio::test]
    async fn borrowed_and_in_flight_sessions_survive_idle_sweeps() {
        let pool = CodexWsPool::default();
        let borrowed = pool.session_at("borrowed", 5).await;
        let in_flight = pool.session_at("in-flight", 5).await;
        let guard = in_flight.lock_owned_at(5).await;

        let borrowed_again = pool.session_at("borrowed", 1_000).await;
        let in_flight_again = pool.session_at("in-flight", 1_000).await;

        assert!(Arc::ptr_eq(&borrowed, &borrowed_again));
        assert!(Arc::ptr_eq(&in_flight, &in_flight_again));
        drop(guard);
    }

    #[test]
    fn connection_reuse_age_remains_fifty_five_minutes() {
        let connected_at = Instant::now();
        assert!(connection_age_is_reusable(
            connected_at,
            connected_at + Duration::from_secs(55 * 60 - 1)
        ));
        assert!(!connection_age_is_reusable(
            connected_at,
            connected_at + Duration::from_secs(55 * 60)
        ));
        assert!(CODEX_WS_MAX_REUSE_AGE > CODEX_WS_IDLE_TTL);
    }

    #[tokio::test]
    async fn eviction_never_replaces_a_borrowed_same_key_session() {
        let pool = Arc::new(CodexWsPool::default());
        let protected = pool.session_at("protected", 0).await;
        let protected_guard = protected.lock_owned_at(0).await;
        for index in 0..(MAX_CODEX_WS_SESSIONS - 1) {
            let _ = pool.session_at(&format!("idle-{index}"), 0).await;
        }

        let _ = pool.session_at("new-session", 1).await;
        let same = pool.session_at("protected", 1).await;

        assert!(Arc::ptr_eq(&protected, &same));
        assert!(pool.sessions.len() <= MAX_CODEX_WS_SESSIONS);
        drop(protected_guard);
    }

    #[test]
    fn reset_clears_continuation_metadata() {
        let mut state = CodexSessionState {
            continuation: Some(CodexContinuation {
                response_id: "resp_1".into(),
                request_fingerprint: "request".into(),
                message_fingerprints: vec!["message".into()],
                assistant_fingerprint: None,
                volatile_context_fingerprint: None,
            }),
            ..Default::default()
        };

        state.reset();
        assert!(state.continuation.is_none());
        assert!(state.connection.is_none());
    }

    #[tokio::test]
    async fn ephemeral_guard_supports_the_no_session_path() {
        let mut guard = CodexSessionGuard::ephemeral().await;
        guard.continuation = Some(CodexContinuation {
            response_id: "response".into(),
            request_fingerprint: "request".into(),
            message_fingerprints: Vec::new(),
            assistant_fingerprint: None,
            volatile_context_fingerprint: None,
        });

        guard.reset();
        assert!(guard.continuation.is_none());
    }
}
