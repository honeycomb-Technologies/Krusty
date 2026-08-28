//! Versioned private IPC for the independently supervised Hive daemon.
//!
//! The protocol deliberately has no dependency on `mitsuro-server` or
//! `mitsuro-core`. That keeps the HTTP server and daemon on opposite sides of a
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
pub use client::{EventSubscription, HiveIpcClient, HiveIpcClientConfig};
pub use error::{AuthError, ClientError, FrameError, PeerError, ProtocolViolation};
pub use frame::{read_frame, write_frame};
#[cfg(unix)]
pub use peer::{current_effective_uid, peer_identity, verify_same_user, PeerIdentity};
pub use types::*;

/// Four-byte frame headers can represent more, but Hive deliberately caps every
/// JSON document at 1 MiB to bound allocation before deserialization.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const PROTOCOL_MAJOR: u16 = 1;
/// First protocol minor that preserves exact provider-aware model identity.
pub const MODEL_IDENTITY_PROTOCOL_MINOR: u16 = 2;
/// First protocol minor that understands group-room commands. Older daemons
/// negotiate a lower minor and the client fails closed with a clear
/// unsupported error instead of silently misrouting a group turn.
pub const GROUP_MESSAGING_PROTOCOL_MINOR: u16 = 3;
/// First protocol minor that atomically creates a Hive Worker, its private
/// conversation, and the Worker's one-time Introduction run.
pub const WORKER_INTRODUCTION_PROTOCOL_MINOR: u16 = 4;
/// First protocol minor that carries an exact, typed user decision for a
/// reviewed Worker Introduction proposal.
pub const WORKER_INTRODUCTION_REVIEW_PROTOCOL_MINOR: u16 = 5;
/// First protocol minor that revision-fences Worker profile and lifecycle
/// mutations inside the daemon runtime authority.
pub const WORKER_LIFECYCLE_PROTOCOL_MINOR: u16 = 6;
/// First protocol minor that returns typed durable acceptance for direct
/// Worker conversation input and never live-steers an ordinary Worker turn.
pub const WORKER_CONVERSATION_PROTOCOL_MINOR: u16 = 7;
/// First protocol minor that revision-fences durable Worker Workflow Goal
/// activation and lifecycle control through the daemon authority.
pub const WORKER_WORKFLOW_PROTOCOL_MINOR: u16 = 8;
/// First protocol minor that carries an exact authenticated-owner decision for
/// one frozen Worker Goal acceptance candidate.
pub const WORKER_GOAL_ACCEPTANCE_PROTOCOL_MINOR: u16 = 9;
/// Exact current-run Stop for a Worker direct conversation ships in the same
/// unreleased v1.9 control-plane slice.
pub const WORKER_CONVERSATION_STOP_PROTOCOL_MINOR: u16 = 9;
/// First protocol minor that grants one short-lived, exact-owner recovery call
/// for a Worker blocked only by unresolved provider-call accounting.
pub const WORKER_GOVERNOR_RECOVERY_PROTOCOL_MINOR: u16 = 10;
/// First protocol minor that archives a group and stops its active turn in
/// one ownership-checked daemon transaction.
pub const GROUP_ARCHIVE_PROTOCOL_MINOR: u16 = 11;
pub const PROTOCOL_MINOR: u16 = GROUP_ARCHIVE_PROTOCOL_MINOR;
pub const IPC_KEY_BYTES: usize = 32;
pub const NONCE_BYTES: usize = 32;
