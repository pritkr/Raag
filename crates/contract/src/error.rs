//! Error taxonomy for Resonance core, auth, playback, and cache layers.

use thiserror::Error;

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

pub type CoreResult<T> = Result<T, CoreError>;
