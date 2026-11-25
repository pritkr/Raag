//! Persistent Offline Downloads:
//! - Path & ID validation (`validate_video_id`, `resolve_confined_path`, `redact_url`)
//! - Atomic `.part` resumable download worker (`DownloadManager`)
//! - `lofty` metadata, cover art, and synced LRC lyrics tagger (`embed_metadata`)

pub mod tagger;
pub mod worker;

use std::path::{Component, Path, PathBuf};

use resonance_contract::{AudioCodec, CoreError, CoreResult, OfflineStorageError};

pub use tagger::embed_metadata;
pub use worker::DownloadManager;

pub const DEFAULT_MAX_OFFLINE_BYTES: u64 = 5 * 1024 * 1024 * 1024; // 5 GB
pub const MAX_SINGLE_TRACK_BYTES: u64 = 512 * 1024 * 1024; // 512 MB DoS guard

/// Validates a YouTube/InnerTube video ID for safe use in filenames and SQLite keys.
/// Enforces `1..=64` ASCII `[A-Za-z0-9_-]` and rejects `..`, `/`, `\`, and NUL.
pub fn validate_video_id(video_id: &str) -> CoreResult<()> {
    if video_id.is_empty()
        || video_id.len() > 64
        || video_id.contains("..")
        || video_id.contains('/')
        || video_id.contains('\\')
        || video_id.contains('\0')
        || !video_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(CoreError::OfflineStorage(
            OfflineStorageError::InvalidVideoId(video_id.to_owned()),
        ));
    }
    Ok(())
}

/// Returns the container extension for a stream codec / MIME type.
pub fn container_ext(codec: AudioCodec, mime_type: &str) -> &'static str {
    if matches!(codec, AudioCodec::Aac) || mime_type.contains("mp4") {
        "m4a"
    } else {
        "webm"
    }
}

/// Deterministic relative filename inside `downloads_root`: `{video_id}_{itag}.{ext}`.
pub fn final_filename(
    video_id: &str,
    itag: u32,
    codec: AudioCodec,
    mime_type: &str,
) -> CoreResult<String> {
    validate_video_id(video_id)?;
    let ext = container_ext(codec, mime_type);
    Ok(format!("{video_id}_{itag}.{ext}"))
}

/// In-progress temporary filename: `{final_name}.part`.
pub fn part_filename(final_name: &str) -> String {
    format!("{final_name}.part")
}

/// Resolves a relative filename strictly inside `root`, rejecting path traversal and symlinks.
pub fn resolve_confined_path(root: &Path, relative_name: &str) -> CoreResult<PathBuf> {
    if relative_name.is_empty() || relative_name.contains('\0') {
        return Err(CoreError::OfflineStorage(
            OfflineStorageError::PathTraversal(relative_name.to_owned()),
        ));
    }

    let rel = Path::new(relative_name);
    let mut comps = rel.components();
    match (comps.next(), comps.next()) {
        (Some(Component::Normal(_)), None) => {}
        _ => {
            return Err(CoreError::OfflineStorage(
                OfflineStorageError::PathTraversal(relative_name.to_owned()),
            ));
        }
    }

    let joined = root.join(rel);
    if let Ok(meta) = std::fs::symlink_metadata(&joined)
        && meta.file_type().is_symlink()
    {
        return Err(CoreError::OfflineStorage(
            OfflineStorageError::PathTraversal(relative_name.to_owned()),
        ));
    }

    let canon_root = std::fs::canonicalize(root)
        .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Io(e.to_string())))?;
    let parent = joined.parent().ok_or_else(|| {
        CoreError::OfflineStorage(OfflineStorageError::PathTraversal(
            relative_name.to_owned(),
        ))
    })?;
    let canon_parent = std::fs::canonicalize(parent)
        .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Io(e.to_string())))?;

    if canon_parent != canon_root {
        return Err(CoreError::OfflineStorage(
            OfflineStorageError::PathTraversal(relative_name.to_owned()),
        ));
    }

    Ok(joined)
}

/// Strips `?query` and `#fragment` from a URL so signed parameters (`sig`, `n`, `pot`, `ip`)
/// never appear in logs, SQLite, or error messages.
pub fn redact_url(raw: &str) -> String {
    if let Ok(mut parsed) = url::Url::parse(raw) {
        parsed.set_query(None);
        parsed.set_fragment(None);
        return parsed.to_string();
    }
    if let Some((base, _)) = raw.split_once('?')
        && (base.starts_with("http://") || base.starts_with("https://"))
    {
        return base.to_owned();
    }
    "<redacted-url>".to_owned()
}

/// Sanitizes an arbitrary error message so embedded URLs/query strings or cookie tokens
/// are redacted before persistence or logging.
pub fn redact_error_message(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for token in raw.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        let trimmed = token.trim_matches(|c| c == '(' || c == ')' || c == '\'' || c == '"');
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            out.push_str(&redact_url(trimmed));
        } else if trimmed.contains("SAPISID")
            || trimmed.contains("sig=")
            || trimmed.contains("pot=")
        {
            out.push_str("<redacted>");
        } else {
            out.push_str(token);
        }
    }
    out
}
