//! First-Class Offline Vault (`crates/core/src/downloads/`).
//!
//! Separates intentional offline downloads and pinned albums/playlists from the
//! transient LRU chunk cache (`audio_chunks`), backed by SQLite state machine
//! (`Queued -> Downloading -> Verified -> Available`) with resumable range fetches
//! and artwork/lyrics bundling.

pub mod quota;
pub mod state;
pub mod vault;

pub use quota::{
    CollectionOfflineProgress, DEFAULT_ESTIMATED_OPUS_BYTES, DEFAULT_ESTIMATED_PREMIUM_BYTES,
    QUOTA_PRESETS_BYTES, QuotaEvaluator, UnpinCleanupReport, compute_collection_progress,
};
pub use state::VaultTrackState;
pub use vault::{OfflineVault, PinnedCollection, PinnedCollectionKind, VaultSummary};
