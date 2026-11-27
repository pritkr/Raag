//! Streaming artifact downloader with live byte progress, fail-closed SHA-256 verification,
//! and atomic `.part -> .verified` promotion.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::StreamExt;
use resonance_contract::UpdateInfo;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::updater::manifest::validate_updater_url;

/// Maximum update artifact size accepted (512 MB safety ceiling).
pub const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

/// Sanitizes a SemVer string into a safe filename component (`[A-Za-z0-9._-]`).
fn sanitize_version_for_filename(version: &str) -> String {
    version
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Streams the artifact to `<staging_dir>/resonance-<version>.part`, validates its SHA-256
/// digest against `info.sha256_hex` (fail-closed if missing), and atomically renames it
/// to `.verified`.
pub async fn download_and_verify_artifact(
    http: &reqwest::Client,
    staging_dir: &Path,
    info: &UpdateInfo,
    allow_loopback_http: bool,
    cancel_flag: Option<Arc<AtomicBool>>,
    on_progress: impl Fn(u64, Option<u64>) + Send + Sync,
) -> Result<PathBuf, String> {
    let url = validate_updater_url(&info.download_url, allow_loopback_http)?;
    let expected_sha = info
        .sha256_hex
        .as_deref()
        .map(str::trim)
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| "update artifact missing valid SHA-256 digest".to_owned())?;

    tokio::fs::create_dir_all(staging_dir)
        .await
        .map_err(|e| format!("failed to create update staging dir: {e}"))?;

    let safe_ver = sanitize_version_for_filename(&info.version);
    let part_path = staging_dir.join(format!("resonance-{safe_ver}.part"));
    let verified_path = staging_dir.join(format!("resonance-{safe_ver}.verified"));

    // Fast path: reuse already verified staged artifact if SHA-256 matches.
    if tokio::fs::try_exists(&verified_path).await.unwrap_or(false)
        && verify_file_sha256(&verified_path, expected_sha).await.is_ok()
    {
        let meta_len = tokio::fs::metadata(&verified_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        on_progress(meta_len, Some(meta_len));
        return Ok(verified_path);
    }

    let resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| format!("artifact download request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "artifact download returned HTTP {}",
            resp.status().as_u16()
        ));
    }

    let total_bytes = resp.content_length();
    if let Some(len) = total_bytes
        && len > MAX_ARTIFACT_BYTES
    {
        return Err("update artifact content-length exceeds 512 MB safety limit".to_owned());
    }

    let mut file = tokio::fs::File::create(&part_path)
        .await
        .map_err(|e| format!("failed to create staging file: {e}"))?;

    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();

    while let Some(chunk_res) = stream.next().await {
        if cancel_flag
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
        {
            drop(file);
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err("update download cancelled".to_owned());
        }
        let chunk = chunk_res.map_err(|e| format!("download stream error: {e}"))?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > MAX_ARTIFACT_BYTES {
            drop(file);
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err("update download exceeded 512 MB safety limit".to_owned());
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("failed writing staging file: {e}"))?;
        on_progress(downloaded, total_bytes.map(|t| t.max(downloaded)));
    }

    file.flush()
        .await
        .map_err(|e| format!("failed flushing staging file: {e}"))?;
    file.sync_all()
        .await
        .map_err(|e| format!("failed syncing staging file: {e}"))?;
    drop(file);

    let actual_sha = hex::encode(hasher.finalize());
    if !actual_sha.eq_ignore_ascii_case(expected_sha) {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(format!(
            "SHA-256 verification failed (expected {expected_sha}, got {actual_sha})"
        ));
    }

    tokio::fs::rename(&part_path, &verified_path)
        .await
        .map_err(|e| format!("failed promoting verified artifact: {e}"))?;

    Ok(verified_path)
}

async fn verify_file_sha256(path: &Path, expected_hex: &str) -> Result<(), String> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("failed reading staged artifact: {e}"))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let actual = hex::encode(hasher.finalize());
    if actual.eq_ignore_ascii_case(expected_hex.trim()) {
        Ok(())
    } else {
        Err("staged artifact SHA-256 mismatch".to_owned())
    }
}
