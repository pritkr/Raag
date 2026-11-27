//! Embedded cover-art extraction & SHA-256 deduplication via `ArtworkCache`.

use lofty::file::TaggedFileExt;
use lofty::picture::PictureType;
use sha2::{Digest, Sha256};

use crate::cache::artwork::ArtworkCache;

/// Maximum embedded artwork payload accepted per track (4 MB zip-bomb guard).
pub const MAX_EMBEDDED_ARTWORK_BYTES: usize = 4 * 1024 * 1024;

/// Cache key prefix used in `ArtworkCache` for deduplicated local album covers.
pub fn local_art_cache_key(art_hash: &str) -> String {
    format!("local_art:{art_hash}")
}

/// Formats the `LoopbackProxy` URL for a cached local artwork hash.
pub fn local_artwork_url(proxy_base_url: &str, art_hash: &str) -> String {
    format!(
        "{}/local-art/{art_hash}",
        proxy_base_url.trim_end_matches('/')
    )
}

/// Extracts `CoverFront` (or first embedded picture) from a `lofty::TaggedFile`,
/// caps it at [`MAX_EMBEDDED_ARTWORK_BYTES`], hashes it with SHA-256, and stores
/// it once in [`ArtworkCache`] so identical album covers are deduplicated.
pub fn extract_and_cache_artwork(
    tagged: &lofty::file::TaggedFile,
    cache: Option<&ArtworkCache>,
) -> Option<String> {
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let pic = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first())?;

    cache_artwork_bytes(pic.data(), cache)
}

/// Hashes raw image bytes (capped at [`MAX_EMBEDDED_ARTWORK_BYTES`]) and deduplicates them
/// into [`ArtworkCache`].
pub fn cache_artwork_bytes(data: &[u8], cache: Option<&ArtworkCache>) -> Option<String> {
    if data.is_empty() || data.len() > MAX_EMBEDDED_ARTWORK_BYTES {
        return None;
    }

    let mut hasher = Sha256::new();
    hasher.update(data);
    let art_hash = hex::encode(hasher.finalize());

    if let Some(artwork_cache) = cache {
        let key = local_art_cache_key(&art_hash);
        if artwork_cache.get(&key, 0).is_none() {
            let _ = artwork_cache.put(&key, 0, data.to_vec());
        }
    }

    Some(art_hash)
}

/// Looks for a sibling album cover image (`cover.jpg`, `cover.png`, `folder.jpg`, `folder.png`,
/// `front.jpg`, `front.png`) next to `audio_path` without following symlinks and honoring
/// [`MAX_EMBEDDED_ARTWORK_BYTES`].
pub fn extract_sibling_cover_artwork(
    audio_path: &std::path::Path,
    cache: Option<&ArtworkCache>,
) -> Option<String> {
    let dir = audio_path.parent()?;
    const CANDIDATES: &[&str] = &[
        "cover.jpg",
        "cover.jpeg",
        "cover.png",
        "folder.jpg",
        "folder.jpeg",
        "folder.png",
        "front.jpg",
        "front.png",
        "Cover.jpg",
        "Folder.jpg",
    ];
    for name in CANDIDATES {
        let candidate = dir.join(name);
        let Ok(meta) = std::fs::symlink_metadata(&candidate) else {
            continue;
        };
        if meta.file_type().is_symlink()
            || !meta.is_file()
            || meta.len() == 0
            || meta.len() > MAX_EMBEDDED_ARTWORK_BYTES as u64
        {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&candidate)
            && let Some(hash) = cache_artwork_bytes(&bytes, cache)
        {
            return Some(hash);
        }
    }
    None
}
