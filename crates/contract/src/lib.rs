//! Frozen Contract Crate for Resonance.
//!
//! Defines domain types, trait boundaries (`MusicProvider`, `AudioBackend`,
//! `AuthStore`, `CacheStore`, `LyricsProvider`, `SponsorBlockProvider`),
//! deterministic `PlayerState` state machine, `EventBus`, error taxonomy,
//! and Wave 0 frozen extension modules (`storage`, `settings`, `catalog_ext`,
//! `rooms`, `local_media`, `updater`, `casting`).

pub mod casting;
pub mod catalog_ext;
pub mod desktop_auth;
pub mod domain;
pub mod error;
pub mod events;
pub mod local_media;
pub mod mutations;
pub mod potoken;
pub mod rooms;
pub mod settings;
pub mod state;
pub mod storage;
pub mod traits;
pub mod updater;

pub use casting::*;
pub use catalog_ext::*;
pub use desktop_auth::*;
pub use domain::*;
pub use error::*;
pub use events::*;
pub use local_media::*;
pub use mutations::*;
pub use potoken::*;
pub use rooms::*;
pub use settings::*;
pub use state::*;
pub use storage::*;
pub use traits::*;
pub use updater::*;
