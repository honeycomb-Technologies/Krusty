use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("I/O error while transferring a protocol frame: {0}")]
    Io(#[from] io::Error),
    #[error("protocol frame header ended after {received} of 4 bytes")]
    TruncatedHeader { received: usize },
    #[error("protocol frame payload ended after {received} of {expected} bytes")]
    TruncatedPayload { expected: usize, received: usize },
    #[error("zero-length protocol frames are invalid")]
    ZeroLength,
    #[error("protocol frame is {actual} bytes; maximum is {maximum}")]
    Oversized { actual: usize, maximum: usize },
    #[error("failed to encode protocol JSON: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("failed to decode protocol JSON: {0}")]
    Decode(#[source] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("IPC key has insecure permissions {mode:o}; expected no group/world access")]
    InsecureKeyPermissions { mode: u32 },
    #[error("IPC key is owned by uid {actual}, not current uid {expected}")]
    WrongKeyOwner { expected: u32, actual: u32 },
    #[error("IPC key is not a regular file")]
    KeyNotRegularFile,
    #[error("IPC key must contain exactly {expected} bytes, found {actual}")]
    InvalidKeyLength { expected: usize, actual: usize },
    #[error("private IPC directory is not a directory")]
    NotDirectory,
    #[error("private IPC directory is owned by uid {actual}, not current uid {expected}")]
    WrongDirectoryOwner { expected: u32, actual: u32 },
    #[error("hello timestamp is outside the allowed clock skew")]
    StaleHello,
    #[error("hello nonce has already been used")]
    ReplayedNonce,
    #[error("hello nonce is malformed")]
    InvalidNonce,
    #[error("hello authentication code is malformed or invalid")]
    InvalidMac,
    #[error("hello client id is empty or too long")]
    InvalidClientId,
    #[error("I/O error while managing IPC authentication material: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Error)]
pub enum PeerError {
    #[error("failed to inspect Unix peer credentials: {0}")]
    Io(#[from] io::Error),
    #[error("Unix peer credential inspection is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("peer uid {actual} does not match current uid {expected}")]
    DifferentUser { expected: u32, actual: u32 },
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProtocolViolation {
    #[error("unsupported protocol version {actual_major}.{actual_minor}; server supports {expected_major}.{expected_minor}")]
    VersionMismatch {
        expected_major: u16,
        expected_minor: u16,
        actual_major: u16,
        actual_minor: u16,
    },
    #[error(
        "protocol command {command} requires minor {required_minor}, peer negotiated minor {actual_minor}"
    )]
    FeatureRequiresMinor {
        command: &'static str,
        required_minor: u16,
        actual_minor: u16,
    },
    #[error("request id is empty or too long")]
    InvalidRequestId,
    #[error("actor identity is empty or too long")]
    InvalidActor,
    #[error("idempotency key is empty or too long")]
    InvalidIdempotencyKey,
    #[error("request deadline has expired")]
    DeadlineExpired,
    #[error("expected {expected} frame, received {actual}")]
    UnexpectedFrame {
        expected: &'static str,
        actual: &'static str,
    },
    #[error("response request id {actual} did not match {expected}")]
    RequestIdMismatch { expected: String, actual: String },
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Peer(#[from] PeerError),
    #[error(transparent)]
    Protocol(#[from] ProtocolViolation),
    #[error("IPC connection timed out")]
    ConnectTimeout,
    #[error("IPC request timed out")]
    RequestTimeout,
    #[error("daemon closed the IPC connection")]
    Closed,
    #[error("daemon rejected the request: {code}: {message}")]
    Remote { code: String, message: String },
    #[error("Unix IPC is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("I/O error while using Hive IPC: {0}")]
    Io(#[from] io::Error),
}
