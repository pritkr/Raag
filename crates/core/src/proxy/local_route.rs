//! Confined `LoopbackProxy` routes for local audio files (`/local/<track_id>`)
//! and deduplicated embedded artwork (`/local-art/<art_hash>`).
//!
//! Security Invariant:
//! - URLs NEVER accept filesystem paths. `/local/<track_id>` strictly accepts
//!   `local:<16-hex>` looked up in SQLite `local_tracks`, and re-verifies that
//!   the canonical path is a regular file confined inside a registered `local_roots`
//!   directory before reading.

use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use hyper::{Request, Response, StatusCode};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::cache::artwork::ArtworkCache;
use crate::local_media::artwork::local_art_cache_key;
use crate::local_media::store::LocalMediaStore;

fn is_valid_local_track_id(id: &str) -> bool {
    let Some(hex_part) = id.strip_prefix("local:") else {
        return false;
    };
    hex_part.len() == 16 && hex_part.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_valid_art_hash(hash: &str) -> bool {
    !hash.is_empty() && hash.len() <= 128 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

fn detect_image_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else {
        "image/jpeg"
    }
}

fn simple_response(status: StatusCode, msg: &'static str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from_static(msg.as_bytes())))
        .unwrap()
}

fn parse_byte_range(header: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let spec = header.strip_prefix("bytes=")?;
    let (start_s, end_s) = spec.split_once('-')?;
    if start_s.is_empty() {
        let suffix: u64 = end_s.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        let start = total.saturating_sub(suffix);
        return Some((start, total - 1));
    }
    let start: u64 = start_s.parse().ok()?;
    let end: u64 = if end_s.is_empty() {
        total - 1
    } else {
        end_s.parse::<u64>().ok()?.min(total - 1)
    };
    if start > end || start >= total {
        return None;
    }
    Some((start, end))
}

/// Handles `/local/<track_id>` and `/local-art/<art_hash>` requests.
/// Returns `None` if the request path does not target local media.
pub async fn handle_local_request(
    req: &Request<Incoming>,
    local_store: Option<&Arc<LocalMediaStore>>,
    artwork: Option<&Arc<ArtworkCache>>,
) -> Option<Response<Full<Bytes>>> {
    let path = req.uri().path();

    if let Some(art_hash) = path.strip_prefix("/local-art/") {
        if !is_valid_art_hash(art_hash) {
            return Some(simple_response(StatusCode::BAD_REQUEST, "invalid art hash"));
        }
        let Some(cache) = artwork else {
            return Some(simple_response(
                StatusCode::NOT_FOUND,
                "artwork cache unavailable",
            ));
        };
        let key = local_art_cache_key(art_hash);
        let Some(bytes) = cache.get(&key, 0) else {
            return Some(simple_response(StatusCode::NOT_FOUND, "artwork not found"));
        };
        let mime = detect_image_mime(&bytes);
        return Some(
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, mime)
                .header(CONTENT_LENGTH, bytes.len())
                .header("Cache-Control", "public, max-age=31536000, immutable")
                .header("X-Resonance-Source", "local-art")
                .body(Full::new(Bytes::copy_from_slice(&bytes)))
                .unwrap(),
        );
    }

    let track_id = path.strip_prefix("/local/")?;
    if !is_valid_local_track_id(track_id) {
        return Some(simple_response(
            StatusCode::BAD_REQUEST,
            "invalid local track id",
        ));
    }

    let Some(store) = local_store else {
        return Some(simple_response(
            StatusCode::NOT_FOUND,
            "local media store unavailable",
        ));
    };

    let Ok(Some(record)) = store.get_track_by_id(track_id) else {
        return Some(simple_response(
            StatusCode::NOT_FOUND,
            "local track not found",
        ));
    };

    let candidate = std::path::Path::new(&record.canonical_path);
    let Ok(sym_meta) = tokio::fs::symlink_metadata(candidate).await else {
        return Some(simple_response(StatusCode::NOT_FOUND, "local file missing"));
    };
    if sym_meta.file_type().is_symlink() || !sym_meta.is_file() {
        return Some(simple_response(
            StatusCode::FORBIDDEN,
            "invalid local file type",
        ));
    }

    let Ok(canon) = std::fs::canonicalize(candidate) else {
        return Some(simple_response(StatusCode::NOT_FOUND, "local file missing"));
    };
    if canon != candidate
        || !store
            .is_path_confined_to_registered_root(&canon)
            .unwrap_or(false)
    {
        return Some(simple_response(
            StatusCode::FORBIDDEN,
            "path escapes registered root",
        ));
    }

    let total = sym_meta.len();
    let is_head = *req.method() == hyper::Method::HEAD;
    let range_header = req.headers().get(RANGE).and_then(|v| v.to_str().ok());

    let (status, start, end) = if let Some(rh) = range_header {
        let Some((s, e)) = parse_byte_range(rh, total) else {
            return Some(
                Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(CONTENT_RANGE, format!("bytes */{total}"))
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            );
        };
        (StatusCode::PARTIAL_CONTENT, s, e)
    } else if total == 0 {
        return Some(
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, record.mime_type.as_str())
                .header(CONTENT_LENGTH, 0)
                .header(ACCEPT_RANGES, "bytes")
                .header("X-Resonance-Source", "local-file")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        );
    } else {
        (StatusCode::OK, 0, total - 1)
    };

    let len = (end - start + 1) as usize;
    let body = if is_head {
        Bytes::new()
    } else {
        let Ok(mut file) = tokio::fs::File::open(&canon).await else {
            return Some(simple_response(
                StatusCode::NOT_FOUND,
                "local file unreadable",
            ));
        };
        if start > 0 && file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
            return Some(simple_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "seek failed",
            ));
        }
        let mut buf = vec![0u8; len];
        if file.read_exact(&mut buf).await.is_err() {
            return Some(simple_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "read failed",
            ));
        }
        Bytes::from(buf)
    };

    let mut builder = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, record.mime_type.as_str())
        .header(CONTENT_LENGTH, len)
        .header(ACCEPT_RANGES, "bytes")
        .header("X-Resonance-Source", "local-file");

    if status == StatusCode::PARTIAL_CONTENT {
        builder = builder.header(CONTENT_RANGE, format!("bytes {start}-{end}/{total}"));
    }

    Some(builder.body(Full::new(body)).unwrap())
}
