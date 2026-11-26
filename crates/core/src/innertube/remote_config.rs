//! Ed25519-signed remote `client_matrix.json` & `solver_manifest.json` hot-updater (`ST-1C`).
//!
//! Fail-closed verification pipeline:
//! 1. Enforce HTTPS (`http://127.0.0.1` / `localhost` allowed only in tests / explicit test builder).
//! 2. Enforce response byte caps (`128 KB` matrix, `32 KB` manifest, `4 MB` solver JS).
//! 3. Verify Ed25519 signature over exact raw response bytes BEFORE JSON parsing.
//! 4. Validate `schema_version == 1`, non-empty enabled clients, monotonic `updated_at_unix_secs`
//!    anti-rollback, and hard-reject any matrix allowing `android_vr` or `ios` to use cookies.
//! 5. Persist atomically (`tmp` -> `sync_all` -> `rename`) under `cache_dir/remote_config/`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub use ed25519_dalek;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use resonance_contract::{CacheStore, CoreError, CoreResult, ProviderError, RemoteConfigStatus};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::innertube::auth::redact_secrets;
use crate::innertube::matrix::ClientMatrix;
use crate::solver::{EjsBundleManifest, QuickJsSolver};
use crate::unix_secs;

pub const DEFAULT_REMOTE_CONFIG_BASE_URL: &str =
    "https://raw.githubusercontent.com/resonance-audio/resonance-config/main";
pub const DEFAULT_CONFIG_ED25519_PUBKEY_HEX: &str =
    match option_env!("RESONANCE_CONFIG_ED25519_PUBKEY_HEX") {
        Some(v) => v,
        None => "0000000000000000000000000000000000000000000000000000000000000000",
    };

const MAX_MATRIX_BYTES: usize = 128 * 1024;
const MAX_MANIFEST_BYTES: usize = 32 * 1024;
const MAX_SOLVER_JS_BYTES: usize = 4 * 1024 * 1024;
const CHECK_TTL_SECS: u64 = 6 * 3600;
const MATRIX_META_KEY: &str = "remote_config:matrix_meta";
const MANIFEST_META_KEY: &str = "remote_config:manifest_meta";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SyncMeta {
    etag: Option<String>,
    last_checked_unix_secs: u64,
}

pub struct RemoteConfigSyncService {
    remote_config_dir: PathBuf,
    cache: Arc<dyn CacheStore>,
    solver: Arc<QuickJsSolver>,
    http: reqwest::Client,
    base_url: String,
    verifying_key: VerifyingKey,
    allow_insecure_localhost: bool,
    status: RwLock<RemoteConfigStatus>,
}

impl RemoteConfigSyncService {
    pub fn new(
        remote_config_dir: PathBuf,
        cache: Arc<dyn CacheStore>,
        solver: Arc<QuickJsSolver>,
        base_url: Option<String>,
    ) -> CoreResult<Self> {
        let verifying_key = parse_verifying_key_hex(DEFAULT_CONFIG_ED25519_PUBKEY_HEX)
            .map_err(|e| CoreError::Provider(ProviderError::Parse(e)))?;
        Self::with_verifying_key(remote_config_dir, cache, solver, base_url, verifying_key)
    }

    pub fn with_verifying_key(
        remote_config_dir: PathBuf,
        cache: Arc<dyn CacheStore>,
        solver: Arc<QuickJsSolver>,
        base_url: Option<String>,
        verifying_key: VerifyingKey,
    ) -> CoreResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .gzip(true)
            .build()
            .map_err(|e| CoreError::Provider(ProviderError::Network(e.to_string())))?;

        let resolved_base = base_url
            .or_else(|| std::env::var("RESONANCE_CONFIG_BASE_URL").ok())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_REMOTE_CONFIG_BASE_URL.to_owned());

        let status = RemoteConfigStatus {
            matrix_revision: 0,
            matrix_source: "embedded".to_owned(),
            solver_bundle_version: solver
                .active_ejs_manifest()
                .map(|m| m.bundle_version)
                .unwrap_or_default(),
            signature_verified: false,
            last_sync_unix_secs: None,
        };

        Ok(Self {
            remote_config_dir,
            cache,
            solver,
            http,
            base_url: resolved_base.trim_end_matches('/').to_owned(),
            verifying_key,
            allow_insecure_localhost: cfg!(test),
            status: RwLock::new(status),
        })
    }

    pub fn with_allow_insecure_localhost(mut self, allow: bool) -> Self {
        self.allow_insecure_localhost = allow;
        self
    }

    pub fn matrix_cache_path(&self) -> PathBuf {
        self.remote_config_dir.join("client_matrix.json")
    }

    pub fn manifest_cache_path(&self) -> PathBuf {
        self.remote_config_dir.join("solver_manifest.json")
    }

    pub fn status(&self) -> RemoteConfigStatus {
        self.status.read().unwrap().clone()
    }

    /// Synchronously loads the last verified cached `ClientMatrix` from `cache_dir/remote_config/`
    /// with 0 ms network wait, falling back to embedded default if absent/corrupt.
    pub fn load_cached_matrix_sync(&self) -> CoreResult<ClientMatrix> {
        let path = self.matrix_cache_path();
        if path.exists()
            && let Ok(raw) = fs::read_to_string(&path)
            && let Ok(matrix) = serde_json::from_str::<ClientMatrix>(&raw)
            && validate_matrix_invariants(&matrix).is_ok()
        {
            let mut st = self.status.write().unwrap();
            st.matrix_revision = matrix.updated_at_unix_secs;
            st.matrix_source = matrix.source.clone();
            return Ok(matrix);
        }
        let embedded = ClientMatrix::embedded_default()?;
        let mut st = self.status.write().unwrap();
        st.matrix_revision = embedded.updated_at_unix_secs;
        st.matrix_source = embedded.source.clone();
        Ok(embedded)
    }

    /// Fetches, verifies, validates, and atomically persists `client_matrix.json`.
    /// Skips network fetch if checked within 6 hours unless `force == true`.
    pub async fn sync_client_matrix(&self, force: bool) -> CoreResult<Option<ClientMatrix>> {
        let now = unix_secs();
        let prev_meta = self.load_sync_meta(MATRIX_META_KEY, now);
        if !force
            && let Some(meta) = &prev_meta
            && now.saturating_sub(meta.last_checked_unix_secs) < CHECK_TTL_SECS
        {
            return Ok(None);
        }

        let matrix_url = format!("{}/client_matrix.json", self.base_url);
        let sig_url = format!("{}/client_matrix.json.sig", self.base_url);
        self.validate_remote_url(&matrix_url)?;
        self.validate_remote_url(&sig_url)?;

        let etag = prev_meta.as_ref().and_then(|m| m.etag.as_deref());
        let Some((raw_bytes, new_etag)) = self
            .fetch_capped_bytes(&matrix_url, MAX_MATRIX_BYTES, etag)
            .await?
        else {
            self.save_sync_meta(
                MATRIX_META_KEY,
                &SyncMeta {
                    etag: prev_meta.and_then(|m| m.etag),
                    last_checked_unix_secs: now,
                },
                now,
            );
            return Ok(None);
        };

        let (sig_bytes, _) = self
            .fetch_capped_bytes(&sig_url, 1024, None)
            .await?
            .ok_or_else(|| {
                CoreError::Provider(ProviderError::Parse("missing matrix .sig".to_owned()))
            })?;

        // Fail-closed: verify Ed25519 signature over exact raw bytes BEFORE JSON parsing.
        verify_ed25519_raw(&self.verifying_key, &raw_bytes, &sig_bytes)?;

        let remote_matrix: ClientMatrix = serde_json::from_slice(&raw_bytes).map_err(|e| {
            CoreError::Provider(ProviderError::Parse(format!(
                "invalid remote client_matrix.json: {e}"
            )))
        })?;

        validate_matrix_invariants(&remote_matrix)?;

        // Anti-rollback monotonic timestamp check.
        let current = self.load_cached_matrix_sync()?;
        if remote_matrix.updated_at_unix_secs < current.updated_at_unix_secs {
            return Err(CoreError::Provider(ProviderError::Parse(format!(
                "remote client_matrix rollback rejected: remote {} < current {}",
                remote_matrix.updated_at_unix_secs, current.updated_at_unix_secs
            ))));
        }

        write_atomic_bytes(&self.matrix_cache_path(), &raw_bytes)?;
        self.save_sync_meta(
            MATRIX_META_KEY,
            &SyncMeta {
                etag: new_etag,
                last_checked_unix_secs: now,
            },
            now,
        );

        let mut st = self.status.write().unwrap();
        st.matrix_revision = remote_matrix.updated_at_unix_secs;
        st.matrix_source = remote_matrix.source.clone();
        st.signature_verified = true;
        st.last_sync_unix_secs = Some(now);

        Ok(Some(remote_matrix))
    }

    /// Fetches, verifies, and hot-swaps `solver_manifest.json` + `yt.solver.*.js` bundle.
    pub async fn sync_solver_manifest(&self, force: bool) -> CoreResult<Option<EjsBundleManifest>> {
        let now = unix_secs();
        let prev_meta = self.load_sync_meta(MANIFEST_META_KEY, now);
        if !force
            && let Some(meta) = &prev_meta
            && now.saturating_sub(meta.last_checked_unix_secs) < CHECK_TTL_SECS
        {
            return Ok(None);
        }

        let manifest_url = format!("{}/solver_manifest.json", self.base_url);
        let sig_url = format!("{}/solver_manifest.json.sig", self.base_url);
        self.validate_remote_url(&manifest_url)?;
        self.validate_remote_url(&sig_url)?;

        let etag = prev_meta.as_ref().and_then(|m| m.etag.as_deref());
        let Some((raw_bytes, new_etag)) = self
            .fetch_capped_bytes(&manifest_url, MAX_MANIFEST_BYTES, etag)
            .await?
        else {
            self.save_sync_meta(
                MANIFEST_META_KEY,
                &SyncMeta {
                    etag: prev_meta.and_then(|m| m.etag),
                    last_checked_unix_secs: now,
                },
                now,
            );
            return Ok(None);
        };

        let (sig_bytes, _) = self
            .fetch_capped_bytes(&sig_url, 1024, None)
            .await?
            .ok_or_else(|| {
                CoreError::Provider(ProviderError::Parse("missing manifest .sig".to_owned()))
            })?;

        verify_ed25519_raw(&self.verifying_key, &raw_bytes, &sig_bytes)?;

        let remote_manifest: EjsBundleManifest =
            serde_json::from_slice(&raw_bytes).map_err(|e| {
                CoreError::Provider(ProviderError::Parse(format!(
                    "invalid remote solver_manifest.json: {e}"
                )))
            })?;

        let current_manifest = self.solver.active_ejs_manifest()?;
        if remote_manifest.bundle_version != current_manifest.bundle_version
            && !remote_manifest.sha256_hex.is_empty()
        {
            self.validate_remote_url(&remote_manifest.updatable_remote_url)?;
            let (bundle_bytes, _) = self
                .fetch_capped_bytes(
                    &remote_manifest.updatable_remote_url,
                    MAX_SOLVER_JS_BYTES,
                    None,
                )
                .await?
                .ok_or_else(|| {
                    CoreError::Solver("empty solver bundle download response".to_owned())
                })?;
            let bundle_js = String::from_utf8(bundle_bytes)
                .map_err(|e| CoreError::Solver(format!("solver bundle invalid utf-8: {e}")))?;

            self.solver.install_ejs_bundle_update(
                &remote_manifest,
                &bundle_js,
                &remote_manifest.sha256_hex,
            )?;
        }

        write_atomic_bytes(&self.manifest_cache_path(), &raw_bytes)?;
        self.save_sync_meta(
            MANIFEST_META_KEY,
            &SyncMeta {
                etag: new_etag,
                last_checked_unix_secs: now,
            },
            now,
        );

        let mut st = self.status.write().unwrap();
        st.solver_bundle_version = remote_manifest.bundle_version.clone();
        st.signature_verified = true;
        st.last_sync_unix_secs = Some(now);

        Ok(Some(remote_manifest))
    }

    fn validate_remote_url(&self, raw_url: &str) -> CoreResult<()> {
        let parsed = Url::parse(raw_url)
            .map_err(|e| CoreError::Provider(ProviderError::Parse(format!("invalid url: {e}"))))?;
        let host = parsed.host_str().unwrap_or_default();
        let is_localhost = matches!(host, "127.0.0.1" | "localhost");
        match parsed.scheme() {
            "https" => Ok(()),
            "http" if self.allow_insecure_localhost && is_localhost => Ok(()),
            scheme => Err(CoreError::Provider(ProviderError::Parse(format!(
                "disallowed remote config scheme: {scheme}"
            )))),
        }
    }

    async fn fetch_capped_bytes(
        &self,
        url: &str,
        max_bytes: usize,
        if_none_match: Option<&str>,
    ) -> CoreResult<Option<(Vec<u8>, Option<String>)>> {
        let mut req = self.http.get(url);
        if let Some(etag) = if_none_match {
            req = req.header("If-None-Match", etag);
        }
        let resp = req.send().await.map_err(|e| {
            CoreError::Provider(ProviderError::Network(redact_secrets(&e.to_string())))
        })?;
        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(CoreError::Provider(ProviderError::Http {
                status: resp.status().as_u16(),
                message: format!("remote config fetch failed for {url}"),
            }));
        }
        if let Some(len) = resp.content_length()
            && len as usize > max_bytes
        {
            return Err(CoreError::Provider(ProviderError::Parse(format!(
                "remote config payload exceeds cap ({len} > {max_bytes})"
            ))));
        }
        let etag = resp
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = resp.bytes().await.map_err(|e| {
            CoreError::Provider(ProviderError::Network(redact_secrets(&e.to_string())))
        })?;
        if bytes.len() > max_bytes {
            return Err(CoreError::Provider(ProviderError::Parse(format!(
                "remote config payload exceeds cap ({} > {max_bytes})",
                bytes.len()
            ))));
        }
        Ok(Some((bytes.to_vec(), etag)))
    }

    fn load_sync_meta(&self, key: &str, now_unix_secs: u64) -> Option<SyncMeta> {
        let (json, _stale) = self.cache.get_metadata(key, now_unix_secs).ok()??;
        serde_json::from_str(&json).ok()
    }

    fn save_sync_meta(&self, key: &str, meta: &SyncMeta, now_unix_secs: u64) {
        if let Ok(json) = serde_json::to_string(meta) {
            let _ = self
                .cache
                .put_metadata(key, &json, CHECK_TTL_SECS, now_unix_secs);
        }
    }
}

pub fn validate_matrix_invariants(matrix: &ClientMatrix) -> CoreResult<()> {
    if matrix.schema_version != 1 {
        return Err(CoreError::Provider(ProviderError::Parse(format!(
            "unsupported client_matrix schema_version: {}",
            matrix.schema_version
        ))));
    }
    if !matrix.clients.iter().any(|c| c.enabled) {
        return Err(CoreError::Provider(ProviderError::Parse(
            "client_matrix must contain at least one enabled client".to_owned(),
        )));
    }
    // Hard security invariant: `android_vr` and `ios` must NEVER be granted cookie permissions.
    for entry in &matrix.clients {
        if matches!(entry.kind.as_str(), "android_vr" | "ios")
            && entry.auth_mode != "anonymous_only"
        {
            return Err(CoreError::Provider(ProviderError::Parse(format!(
                "security invariant violation: {} cannot have auth_mode={}",
                entry.kind, entry.auth_mode
            ))));
        }
    }
    Ok(())
}

pub fn parse_verifying_key_hex(hex_str: &str) -> Result<VerifyingKey, String> {
    let bytes = hex::decode(hex_str.trim()).map_err(|e| format!("invalid pubkey hex: {e}"))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "Ed25519 pubkey must be 32 bytes".to_owned())?;
    VerifyingKey::from_bytes(&arr).map_err(|e| format!("invalid Ed25519 pubkey: {e}"))
}

fn verify_ed25519_raw(key: &VerifyingKey, payload: &[u8], sig_raw: &[u8]) -> CoreResult<()> {
    let sig_bytes: [u8; 64] = if sig_raw.len() == 64 {
        sig_raw
            .try_into()
            .map_err(|_| CoreError::Provider(ProviderError::Parse("bad signature length".into())))?
    } else {
        let text = std::str::from_utf8(sig_raw)
            .map_err(|_| CoreError::Provider(ProviderError::Parse("invalid .sig encoding".into())))?
            .trim();
        let decoded = hex::decode(text).map_err(|e| {
            CoreError::Provider(ProviderError::Parse(format!("invalid .sig hex: {e}")))
        })?;
        decoded.try_into().map_err(|_| {
            CoreError::Provider(ProviderError::Parse(
                "Ed25519 signature must be 64 bytes".to_owned(),
            ))
        })?
    };

    let signature = Signature::from_bytes(&sig_bytes);
    key.verify_strict(payload, &signature).map_err(|e| {
        CoreError::Provider(ProviderError::Parse(format!(
            "Ed25519 signature verification failed: {e}"
        )))
    })
}

fn write_atomic_bytes(path: &Path, bytes: &[u8]) -> CoreResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            CoreError::Cache(resonance_contract::CacheError::Io(format!(
                "create remote_config dir: {e}"
            )))
        })?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = fs::File::create(&tmp).map_err(|e| {
        CoreError::Cache(resonance_contract::CacheError::Io(format!(
            "create remote_config tmp: {e}"
        )))
    })?;
    f.write_all(bytes).map_err(|e| {
        CoreError::Cache(resonance_contract::CacheError::Io(format!(
            "write remote_config tmp: {e}"
        )))
    })?;
    f.sync_all().map_err(|e| {
        CoreError::Cache(resonance_contract::CacheError::Io(format!(
            "sync remote_config tmp: {e}"
        )))
    })?;
    fs::rename(&tmp, path).map_err(|e| {
        CoreError::Cache(resonance_contract::CacheError::Io(format!(
            "rename remote_config file: {e}"
        )))
    })?;
    Ok(())
}
