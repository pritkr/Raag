//! H3.2 Local-File Import (`lofty` header-only scanner + SQLite + ArtworkCache).
//!
//! Local audio files become first-class `Track` items with deterministic IDs
//! (`local:<16-hex-sha256>`) served through `LoopbackProxy` at `/local/<track_id>`.

pub mod artwork;
pub mod scanner;
pub mod store;

pub use artwork::{MAX_EMBEDDED_ARTWORK_BYTES, extract_and_cache_artwork, local_artwork_url};
pub use scanner::{
    MAX_LOCAL_AUDIO_FILE_BYTES, compute_local_track_id, scan_all_roots, scan_root,
    supported_mime_for_extension,
};
pub use store::LocalMediaStore;
