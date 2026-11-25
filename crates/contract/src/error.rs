//! Error taxonomy for Resonance core, auth, playback, and cache layers.

use thiserror::Error;

use crate::{MutationError, PoTokenError};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CoreError {
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
    #[error("playback error: {0}")]
    Playback(#[from] PlaybackError),
    #[error("auth error: {0}")]
    Auth(#[from] AuthError),
    #[error("cache error: {0}")]
    Cache(#[from] CacheError),
    #[error("offline storage error: {0}")]
    OfflineStorage(#[from] OfflineStorageError),
    #[error("po token error: {0}")]
    PoToken(#[from] PoTokenError),
    #[error("mutation error: {0}")]
    Mutation(#[from] MutationError),
    #[error("solver error: {0}")]
    Solver(String),
    #[error("invalid state transition: {from} -> {to}")]
    InvalidTransition { from: String, to: String },
    #[error("offline: {0}")]
    Offline(String),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProviderError {
    #[error("network request failed: {0}")]
    Network(String),
    #[error("http status {status}: {message}")]
    Http { status: u16, message: String },
    #[error("rate limited (429), retry after {retry_after_secs:?}s")]
    RateLimited { retry_after_secs: Option<u64> },
    #[error("unplayable ({status}): {reason}")]
    Unplayable { status: String, reason: String },
    #[error("parse error: {0}")]
    Parse(String),
    #[error("no suitable audio stream found for {video_id}")]
    NoStream { video_id: String },
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AuthError {
    #[error("not signed in")]
    NotSignedIn,
    #[error("session revoked or expired (401)")]
    Revoked,
    #[error("missing required cookie: {0}")]
    MissingCookie(String),
    #[error("client auth isolation violation: {0}")]
    IsolationViolation(String),
    #[error("keystore error: {0}")]
    Keystore(String),
    #[error("os keyring unavailable ({0}); fell back to 0600 file store")]
    KeyringUnavailable(String),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PlaybackError {
    #[error("audio backend unavailable: {0}")]
    BackendUnavailable(String),
    #[error("stream expired (403) and recovery failed: {0}")]
    StreamExpired(String),
    #[error("decode failure: {0}")]
    Decode(String),
    #[error("empty queue")]
    EmptyQueue,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CacheError {
    #[error("io error: {0}")]
    Io(String),
    #[error("sqlite error: {0}")]
    Sqlite(String),
    #[error("corrupt cache entry healed: {0}")]
    CorruptHealed(String),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OfflineStorageError {
    #[error("invalid video id for offline storage: {0}")]
    InvalidVideoId(String),
    #[error("path traversal rejected: {0}")]
    PathTraversal(String),
    #[error("offline quota exceeded (used {used_bytes}, limit {limit_bytes})")]
    QuotaExceeded { used_bytes: u64, limit_bytes: u64 },
    #[error(
        "insufficient free disk space (available {available_bytes}, required {required_bytes})"
    )]
    InsufficientSpace {
        available_bytes: u64,
        required_bytes: u64,
    },
    #[error("download cancelled for {video_id}")]
    Cancelled { video_id: String },
    #[error("download not found: {video_id}")]
    NotFound { video_id: String },
    #[error("integrity mismatch for {video_id}: expected {expected}, got {actual}")]
    IntegrityMismatch {
        video_id: String,
        expected: String,
        actual: String,
    },
    #[error("tagging failed: {0}")]
    Tagging(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("sqlite error: {0}")]
    Sqlite(String),
}

pub type CoreResult<T> = Result<T, CoreError>;
