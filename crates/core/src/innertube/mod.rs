//! InnerTube protocol implementation with data-driven client matrix, decoder-driven format
//! selection, PO-token policy, and strict per-client auth isolation.

pub mod auth;
pub mod client;
pub mod clients;
pub mod formats;
pub mod matrix;
pub mod parser;
pub mod provider_adapter;

pub use auth::{FileAuthStore, compute_sapisidhash, sanitize_android_webview_ua};
pub use client::InnerTubeClient;
pub use clients::{AuthMode, ClientConfig, ClientKind};
pub use formats::TargetPlatform;
pub use matrix::{AccountTier, ClientMatrix, ClientMatrixEntry, PoTokenPolicy};
pub use provider_adapter::{BrowseDelegate, CompositeMusicProvider, PlaybackResolver};
