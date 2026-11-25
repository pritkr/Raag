//! Decomposed Single Source-of-Truth Playback Engine:
//! - [`fsm`] — pure deterministic `PlaybackFsm` state transitions (100% unit-testable, zero I/O)
//! - [`queue`] — pure `QueueManager` (shuffle, repeat, up-next, reorder, radio dedup)
//! - [`prefetch`] — 50% next-track stream pre-resolution + 75% Proxy v2 dual-buffer warm-up
//! - [`controller`] — async Tokio bridge [`PlaybackController`]

pub mod controller;
pub mod fsm;
pub mod prefetch;
pub mod queue;

pub use controller::{AudioTelemetry, PlaybackController};
pub use fsm::{PlaybackFsm, TickEffects};
pub use prefetch::{
    NEXT_TRACK_PREWARM_CHUNKS, PrefetchCoordinator, PrefetchDecision, PrefetchThresholds,
};
pub use queue::QueueManager;
