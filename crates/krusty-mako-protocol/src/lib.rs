//! Versioned private IPC for the independently supervised Mako daemon.
//!
//! The protocol deliberately has no dependency on `krusty-server` or
//! `krusty-core`. That keeps the HTTP server and daemon on opposite sides of a
//! small, typed transport boundary instead of creating a dependency cycle.
//! Every JSON document is preceded by a four-byte unsigned big-endian length.

mod auth;
mod client;
mod error;
mod frame;
#[cfg(unix)]
mod peer;
mod types;

pub use auth::{ensure_private_dir, unix_time_millis, AuthPolicy, IpcKey, NonceReplayGuard};
pub use client::{EventSubscription, MakoIpcClient, MakoIpcClientConfig};
pub use error::{AuthError, ClientError, FrameError, PeerError, ProtocolViolation};
pub use frame::{read_frame, write_frame};
#[cfg(unix)]
pub use peer::{current_effective_uid, peer_identity, verify_same_user, PeerIdentity};
pub use types::*;

/// Four-byte frame headers can represent more, but Mako deliberately caps every
/// JSON document at 1 MiB to bound allocation before deserialization.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const PROTOCOL_MAJOR: u16 = 1;
/// First protocol minor that preserves exact provider-aware model identity.
pub const MODEL_IDENTITY_PROTOCOL_MINOR: u16 = 2;
pub const PROTOCOL_MINOR: u16 = MODEL_IDENTITY_PROTOCOL_MINOR;
pub const IPC_KEY_BYTES: usize = 32;
pub const NONCE_BYTES: usize = 32;
