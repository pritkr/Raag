//! First-Class Offline Vault (`crates/core/src/downloads/`).
//!
//! Separates intentional offline downloads and pinned albums/playlists from the
//! transient LRU chunk cache (`audio_chunks`), backed by SQLite state machine
//! (`Queued -> Downloading -> Verified -> Available`) with resumable range fetches
//! and artwork/lyrics bundling.

pub mod state;
pub mod vault;

pub use state::VaultTrackState;
pub use vault::{OfflineVault, PinnedCollection, PinnedCollectionKind, VaultSummary};
