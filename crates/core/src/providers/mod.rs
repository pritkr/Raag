//! Hybrid `ytmapi-rs` + `rustypipe` + `InnerTubeClient` provider engine
//! and automatic ATR watchtime reporter.

pub mod atr_reporter;
pub mod hybrid;
pub mod mappers;

#[cfg(feature = "provider-rustypipe")]
pub mod rustypipe_adapter;

#[cfg(feature = "provider-ytmapi")]
pub mod ytmapi_adapter;

pub use atr_reporter::{AtrReporter, AtrTracker, generate_cpn};
pub use hybrid::{HybridMusicProvider, refresh_library_snapshot};

#[cfg(feature = "provider-rustypipe")]
pub use rustypipe_adapter::{
    RustyPipeBrowseAdapter, RustyPipePlaybackAdapter, select_rustypipe_stream,
};

#[cfg(feature = "provider-ytmapi")]
pub use ytmapi_adapter::YtmapiBrowseAdapter;
