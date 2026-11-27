//! High-speed incremental `lofty` header-only local audio scanner.
//!
//! Runs strictly on `tokio::task::spawn_blocking` (zero UI thread stalls),
//! enforces symlink/path-traversal confinement inside registered `local_roots`,
//! skips unchanged `(file_size_bytes, mtime_unix_secs)` entries, deduplicates
//! embedded cover art via `ArtworkCache`, and prunes deleted files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use lofty::file::AudioFile;
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey};
use resonance_contract::{
    AlbumRef, ArtistRef, CoreResult, LocalScanProgress, LocalTrackRecord, Track,
};
use sha2::{Digest, Sha256};

use crate::cache::artwork::ArtworkCache;
use crate::local_media::artwork::{
    extract_and_cache_artwork, extract_sibling_cover_artwork, local_artwork_url,
};
use crate::local_media::store::LocalMediaStore;
use crate::unix_secs;

/// Maximum local audio file size indexed (2 GB safety cap).
pub const MAX_LOCAL_AUDIO_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Maps supported audio file extensions to their canonical MIME type.
pub fn supported_mime_for_extension(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "flac" => Some("audio/flac"),
        "mp3" => Some("audio/mpeg"),
        "m4a" | "mp4" | "aac" => Some("audio/mp4"),
        "ogg" | "opus" => Some("audio/ogg"),
        "wav" => Some("audio/wav"),
        _ => None,
    }
}

/// Deterministic `local:<16-hex-sha256>` track ID from canonical file path.
pub fn compute_local_track_id(canonical_path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_path.as_bytes());
    let hex_digest = hex::encode(hasher.finalize());
    format!("local:{}", &hex_digest[..16])
}

/// Scans all registered `local_roots` on a blocking worker thread.
pub async fn scan_all_roots(
    store: Arc<LocalMediaStore>,
    artwork: Option<Arc<ArtworkCache>>,
    proxy_base_url: String,
    on_progress: Option<Arc<dyn Fn(LocalScanProgress) + Send + Sync>>,
) -> CoreResult<LocalScanProgress> {
    tokio::task::spawn_blocking(move || {
        let roots = store.list_roots()?;
        let mut total = LocalScanProgress {
            scanning: true,
            ..Default::default()
        };
        for root in roots {
            let sub = scan_root_blocking(
                &store,
                artwork.as_deref(),
                Path::new(&root.path),
                &proxy_base_url,
                on_progress.as_deref(),
            )?;
            total.files_seen += sub.files_seen;
            total.files_indexed += sub.files_indexed;
            total.files_skipped_unchanged += sub.files_skipped_unchanged;
            total.files_removed_missing += sub.files_removed_missing;
            total.errors += sub.errors;
        }
        total.scanning = false;
        if let Some(cb) = on_progress.as_deref() {
            cb(total.clone());
        }
        Ok(total)
    })
    .await
    .unwrap_or_else(|e| {
        Err(resonance_contract::CoreError::Cache(
            resonance_contract::CacheError::Io(e.to_string()),
        ))
    })
}

/// Scans a single root directory on a blocking worker thread.
pub async fn scan_root(
    store: Arc<LocalMediaStore>,
    artwork: Option<Arc<ArtworkCache>>,
    root_path: PathBuf,
    proxy_base_url: String,
    on_progress: Option<Arc<dyn Fn(LocalScanProgress) + Send + Sync>>,
) -> CoreResult<LocalScanProgress> {
    tokio::task::spawn_blocking(move || {
        let mut progress = scan_root_blocking(
            &store,
            artwork.as_deref(),
            &root_path,
            &proxy_base_url,
            on_progress.as_deref(),
        )?;
        progress.scanning = false;
        if let Some(cb) = on_progress.as_deref() {
            cb(progress.clone());
        }
        Ok(progress)
    })
    .await
    .unwrap_or_else(|e| {
        Err(resonance_contract::CoreError::Cache(
            resonance_contract::CacheError::Io(e.to_string()),
        ))
    })
}

fn scan_root_blocking(
    store: &LocalMediaStore,
    artwork: Option<&ArtworkCache>,
    root_path: &Path,
    proxy_base_url: &str,
    on_progress: Option<&(dyn Fn(LocalScanProgress) + Send + Sync)>,
) -> CoreResult<LocalScanProgress> {
    let canonical_root = std::fs::canonicalize(root_path).map_err(|e| {
        resonance_contract::CoreError::Cache(resonance_contract::CacheError::Io(e.to_string()))
    })?;
    let root_str = canonical_root.to_string_lossy().into_owned();

    let mut progress = LocalScanProgress {
        scanning: true,
        ..Default::default()
    };
    let mut seen_paths = HashSet::<String>::new();
    let mut visited_dirs = HashSet::<PathBuf>::new();
    let mut stack = vec![canonical_root.clone()];
    let mut pending_batch: Vec<(LocalTrackRecord, Option<String>)> = Vec::with_capacity(100);
    let mut last_emit = Instant::now();

    while let Some(dir) = stack.pop() {
        if !visited_dirs.insert(dir.clone()) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            progress.errors += 1;
            continue;
        };

        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();

            // Canonicalize and strictly confine inside `canonical_root` (blocks symlink escapes).
            let Ok(canonical_entry) = std::fs::canonicalize(&path) else {
                continue;
            };
            if !canonical_entry.starts_with(&canonical_root) {
                continue;
            }

            let Ok(meta) = std::fs::metadata(&canonical_entry) else {
                continue;
            };

            if meta.is_dir() {
                stack.push(canonical_entry);
                continue;
            }
            if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_LOCAL_AUDIO_FILE_BYTES {
                continue;
            }

            let Some(ext) = canonical_entry.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            let Some(mime_type) = supported_mime_for_extension(ext) else {
                continue;
            };

            progress.files_seen += 1;
            let canon_str = canonical_entry.to_string_lossy().into_owned();
            seen_paths.insert(canon_str.clone());

            let file_size = meta.len();
            let mtime_secs = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            if let Ok(Some((prev_size, prev_mtime, _))) = store.get_file_fingerprint(&canon_str)
                && prev_size == file_size
                && prev_mtime == mtime_secs
            {
                progress.files_skipped_unchanged += 1;
                maybe_emit_progress(&progress, on_progress, &mut last_emit);
                continue;
            }

            match index_single_file(
                &canonical_entry,
                &canon_str,
                &root_str,
                file_size,
                mtime_secs,
                mime_type,
                artwork,
                proxy_base_url,
            ) {
                Ok((record, art_hash)) => {
                    pending_batch.push((record, art_hash));
                    progress.files_indexed += 1;
                    if pending_batch.len() >= 100 {
                        store.upsert_tracks_batch(&pending_batch, unix_secs())?;
                        pending_batch.clear();
                    }
                }
                Err(_) => {
                    progress.errors += 1;
                }
            }

            maybe_emit_progress(&progress, on_progress, &mut last_emit);
        }
    }

    if !pending_batch.is_empty() {
        store.upsert_tracks_batch(&pending_batch, unix_secs())?;
    }

    let removed = store.prune_missing_under_root(&root_str, &seen_paths)?;
    progress.files_removed_missing = removed;
    store.mark_root_scanned(&root_str, unix_secs())?;

    Ok(progress)
}

fn maybe_emit_progress(
    progress: &LocalScanProgress,
    on_progress: Option<&(dyn Fn(LocalScanProgress) + Send + Sync)>,
    last_emit: &mut Instant,
) {
    let Some(cb) = on_progress else {
        return;
    };
    if progress.files_seen.is_multiple_of(25) || last_emit.elapsed() >= Duration::from_millis(150) {
        cb(progress.clone());
        *last_emit = Instant::now();
    }
}

#[allow(clippy::too_many_arguments)]
fn index_single_file(
    path: &Path,
    canonical_path: &str,
    root_path: &str,
    file_size_bytes: u64,
    mtime_unix_secs: u64,
    mime_type: &str,
    artwork: Option<&ArtworkCache>,
    proxy_base_url: &str,
) -> Result<(LocalTrackRecord, Option<String>), String> {
    let probe = Probe::open(path).map_err(|e| e.to_string())?;
    let probe = probe.guess_file_type().map_err(|e| e.to_string())?;
    let tagged = probe.read().map_err(|e| e.to_string())?;

    let props = tagged.properties();
    let duration_ms = props.duration().as_millis() as u64;
    let sample_rate_hz = props.sample_rate();
    let bitrate_kbps = props.audio_bitrate();

    let tag = lofty::file::TaggedFileExt::primary_tag(&tagged)
        .or_else(|| lofty::file::TaggedFileExt::first_tag(&tagged));

    let fallback_title = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Unknown Track")
        .to_owned();

    let title = tag
        .and_then(|t| t.title())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_title);

    let artist_name = tag
        .and_then(|t| t.artist())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Unknown Artist".to_owned());

    let album_ref = tag
        .and_then(|t| t.album())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .map(|title| AlbumRef { id: None, title });

    let embedded_lyrics = tag
        .and_then(|t| t.get_string(&ItemKey::Lyrics))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| read_sidecar_lrc(path));

    let art_hash = extract_and_cache_artwork(&tagged, artwork)
        .or_else(|| extract_sibling_cover_artwork(path, artwork));
    let artwork_url = art_hash
        .as_deref()
        .map(|h| local_artwork_url(proxy_base_url, h));

    let track_id = compute_local_track_id(canonical_path);
    let track = Track {
        id: track_id.clone(),
        title,
        artists: vec![ArtistRef {
            id: None,
            name: artist_name,
        }],
        album: album_ref,
        duration_ms,
        artwork_url,
        is_explicit: false,
        is_liked: false,
        is_video: false,
    };

    let record = LocalTrackRecord {
        track_id,
        canonical_path: canonical_path.to_owned(),
        root_path: root_path.to_owned(),
        file_size_bytes,
        mtime_unix_secs,
        mime_type: mime_type.to_owned(),
        sample_rate_hz,
        bitrate_kbps,
        track,
        embedded_lyrics,
    };

    Ok((record, art_hash))
}

/// Reads an optional sidecar `.lrc` file next to the audio file (capped at 512 KB).
fn read_sidecar_lrc(audio_path: &Path) -> Option<String> {
    let lrc_path = audio_path.with_extension("lrc");
    let meta = std::fs::symlink_metadata(&lrc_path).ok()?;
    if meta.file_type().is_symlink()
        || !meta.is_file()
        || meta.len() == 0
        || meta.len() > 512 * 1024
    {
        return None;
    }
    std::fs::read_to_string(&lrc_path)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}
