//! Frozen Phase 0 Contract Crate for Resonance.
//!
//! Defines domain types, trait boundaries (`MusicProvider`, `AudioBackend`,
//! `AuthStore`, `CacheStore`, `LyricsProvider`, `SponsorBlockProvider`),
//! deterministic `PlayerState` state machine, `EventBus`, and error taxonomy.

pub mod desktop_auth;
pub mod domain;
pub mod error;
pub mod events;
pub mod mutations;
pub mod potoken;
pub mod state;
pub mod traits;

pub use desktop_auth::*;
pub use domain::*;
pub use error::*;
pub use events::*;
pub use mutations::*;
pub use potoken::*;
pub use state::*;
pub use traits::*;
