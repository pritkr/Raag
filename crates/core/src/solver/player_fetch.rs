//! Live YouTube `base.js` player URL extraction, single-flight download,
//! host allowlisting, 6-hour disk caching, and bounded bundle eviction.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use resonance_contract::{CoreError, CoreResult};
use url::Url;

use crate::innertube::auth::redact_secrets;

const ALLOWED_PLAYER_HOSTS: &[&str] = &["music.youtube.com", "www.youtube.com", "s.ytimg.com"];
const MAX_PLAYER_JS_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_PLAYER_TTL: Duration = Duration::from_secs(6 * 3600);
const DEFAULT_MAX_BUNDLES: usize = 3;
const DEFAULT_DISCOVERY_URL: &str = "https://music.youtube.com/iframe_api";

#[derive(Debug, Clone)]
pub struct FetchedPlayerScript {
    pub player_hash: String,
    pub player_url: String,
    pub raw_path: PathBuf,
    pub source: String,
}

pub struct PlayerScriptFetcher {
    js_solver_dir: PathBuf,
    http: reqwest::Client,
    discovery_url: String,
    ttl: Duration,
    max_bundles: usize,
    allow_localhost: bool,
    in_flight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl PlayerScriptFetcher {
    pub fn new(js_solver_dir: PathBuf) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .gzip(true)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            js_solver_dir,
            http,
            discovery_url: DEFAULT_DISCOVERY_URL.to_owned(),
            ttl: DEFAULT_PLAYER_TTL,
            max_bundles: DEFAULT_MAX_BUNDLES,
            allow_localhost: cfg!(test),
            in_flight: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_discovery_url(mut self, url: impl Into<String>) -> Self {
        self.discovery_url = url.into();
        self
    }

    pub fn with_allow_localhost(mut self, allow: bool) -> Self {
        self.allow_localhost = allow;
        self
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn with_max_bundles(mut self, max_bundles: usize) -> Self {
        self.max_bundles = max_bundles.max(1);
        self
    }

    pub fn raw_script_path(&self, player_hash: &str) -> PathBuf {
        self.js_solver_dir.join(format!("raw_{player_hash}.js"))
    }

    pub fn preprocessed_script_path(&self, player_hash: &str) -> PathBuf {
        self.js_solver_dir.join(format!("pre_{player_hash}.js"))
    }

    /// Invalidates cached `raw_<hash>.js` and `pre_<hash>.js` (e.g. on HTTP 403).
    pub fn invalidate(&self, player_hash: &str) {
        let _ = fs::remove_file(self.raw_script_path(player_hash));
        let _ = fs::remove_file(self.preprocessed_script_path(player_hash));
    }

    /// Extracts `(player_hash, player_url_path)` matching `/s/player/([0-9a-fA-F]{8,16})/[^"']*base\.js`.
    pub fn extract_player_path(html_or_js: &str) -> Option<(String, String)> {
        let mut search_from = 0usize;
        let marker = "/s/player/";
        while let Some(rel_idx) = html_or_js[search_from..].find(marker) {
            let start = search_from + rel_idx;
            let after_marker = &html_or_js[start + marker.len()..];
            if let Some(slash_idx) = after_marker.find('/') {
                let hash = &after_marker[..slash_idx];
                if (8..=16).contains(&hash.len()) && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    let rest = &html_or_js[start..];
                    let end = rest
                        .find(['"', '\'', '\\', '<', '>', ' ', '\n', '\r', '\t'])
                        .unwrap_or(rest.len());
                    let candidate = &rest[..end];
                    if candidate.ends_with("base.js") {
                        return Some((hash.to_owned(), candidate.to_owned()));
                    }
                }
            }
            search_from = start + marker.len();
        }
        None
    }

    /// Discovers the active player URL from `discovery_url` (`music.youtube.com/iframe_api` or `/`).
    pub async fn discover_active_player_url(&self) -> CoreResult<(String, String)> {
        self.validate_url(&self.discovery_url)?;
        let resp = self
            .http
            .get(&self.discovery_url)
            .send()
            .await
            .map_err(|e| CoreError::Solver(redact_secrets(&format!("player discovery: {e}"))))?;
        if !resp.status().is_success() {
            return Err(CoreError::Solver(format!(
                "player discovery HTTP {}",
                resp.status()
            )));
        }
        let body = read_capped_text(resp, MAX_PLAYER_JS_BYTES, "player discovery").await?;
        let (hash, path) = Self::extract_player_path(&body).ok_or_else(|| {
            CoreError::Solver("failed to extract /s/player/<hash>/.../base.js URL".to_owned())
        })?;
        let full_url = resolve_player_url(&self.discovery_url, &path)?;
        self.validate_url(&full_url)?;
        Ok((hash, full_url))
    }

    /// Fetches and caches `raw_<player_hash>.js` with single-flight locking and 6h TTL.
    pub async fn fetch_player_script(
        &self,
        player_hash: &str,
        player_url: &str,
    ) -> CoreResult<FetchedPlayerScript> {
        self.validate_url(player_url)?;
        let lock = {
            let mut map = self.in_flight.lock().unwrap();
            map.entry(player_hash.to_owned())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;

        let raw_path = self.raw_script_path(player_hash);
        if is_fresh_file(&raw_path, self.ttl)
            && let Ok(source) = fs::read_to_string(&raw_path)
        {
            return Ok(FetchedPlayerScript {
                player_hash: player_hash.to_owned(),
                player_url: player_url.to_owned(),
                raw_path,
                source,
            });
        }

        let resp = self
            .http
            .get(player_url)
            .send()
            .await
            .map_err(|e| CoreError::Solver(redact_secrets(&format!("fetch base.js: {e}"))))?;
        if !resp.status().is_success() {
            return Err(CoreError::Solver(format!(
                "fetch base.js HTTP {}",
                resp.status()
            )));
        }
        let source = read_capped_text(resp, MAX_PLAYER_JS_BYTES, "base.js").await?;
        write_atomic(&raw_path, source.as_bytes())?;
        evict_old_bundles_in_dir(&self.js_solver_dir, self.max_bundles);

        Ok(FetchedPlayerScript {
            player_hash: player_hash.to_owned(),
            player_url: player_url.to_owned(),
            raw_path,
            source,
        })
    }

    /// Convenience helper: discovers the active player URL and fetches `base.js`.
    pub async fn fetch_active_player(&self) -> CoreResult<FetchedPlayerScript> {
        let (hash, url) = self.discover_active_player_url().await?;
        self.fetch_player_script(&hash, &url).await
    }

    fn validate_url(&self, raw_url: &str) -> CoreResult<()> {
        let parsed = Url::parse(raw_url)
            .map_err(|e| CoreError::Solver(format!("invalid player url: {e}")))?;
        let host = parsed.host_str().unwrap_or_default();
        let is_localhost = matches!(host, "127.0.0.1" | "localhost");
        match parsed.scheme() {
            "https" => {
                if ALLOWED_PLAYER_HOSTS.contains(&host) || (self.allow_localhost && is_localhost) {
                    Ok(())
                } else {
                    Err(CoreError::Solver(format!(
                        "disallowed player script host: {host}"
                    )))
                }
            }
            "http" if self.allow_localhost && is_localhost => Ok(()),
            scheme => Err(CoreError::Solver(format!(
                "disallowed player script scheme: {scheme}"
            ))),
        }
    }
}

/// Evicts oldest `raw_<hash>.js` / `pre_<hash>.js` pairs in `js_solver_dir`, keeping `keep_latest`.
pub fn evict_old_bundles_in_dir(js_solver_dir: &Path, keep_latest: usize) {
    let Ok(entries) = fs::read_dir(js_solver_dir) else {
        return;
    };
    let mut by_hash: HashMap<String, SystemTime> = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let hash = name
            .strip_prefix("raw_")
            .or_else(|| name.strip_prefix("pre_"))
            .and_then(|rest| rest.strip_suffix(".js"));
        let Some(hash) = hash else {
            continue;
        };
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        by_hash
            .entry(hash.to_owned())
            .and_modify(|prev| {
                if mtime > *prev {
                    *prev = mtime;
                }
            })
            .or_insert(mtime);
    }

    if by_hash.len() <= keep_latest {
        return;
    }
    let mut sorted: Vec<(String, SystemTime)> = by_hash.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    for (hash, _) in sorted.into_iter().skip(keep_latest) {
        let _ = fs::remove_file(js_solver_dir.join(format!("raw_{hash}.js")));
        let _ = fs::remove_file(js_solver_dir.join(format!("pre_{hash}.js")));
    }
}

fn resolve_player_url(discovery_url: &str, path: &str) -> CoreResult<String> {
    if path.starts_with("https://") || path.starts_with("http://") {
        return Ok(path.to_owned());
    }
    let base = Url::parse(discovery_url)
        .map_err(|e| CoreError::Solver(format!("invalid discovery url: {e}")))?;
    base.join(path)
        .map(|u| u.to_string())
        .map_err(|e| CoreError::Solver(format!("failed to resolve player url: {e}")))
}

fn is_fresh_file(path: &Path, ttl: Duration) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age < ttl)
        .unwrap_or(false)
}

pub(crate) fn write_atomic(target: &Path, bytes: &[u8]) -> CoreResult<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| CoreError::Solver(format!("create solver dir: {e}")))?;
    }
    let tmp = target.with_extension("js.tmp");
    let mut f = fs::File::create(&tmp)
        .map_err(|e| CoreError::Solver(format!("create tmp solver file: {e}")))?;
    f.write_all(bytes)
        .map_err(|e| CoreError::Solver(format!("write tmp solver file: {e}")))?;
    f.sync_all()
        .map_err(|e| CoreError::Solver(format!("sync tmp solver file: {e}")))?;
    fs::rename(&tmp, target)
        .map_err(|e| CoreError::Solver(format!("rename tmp solver file: {e}")))?;
    Ok(())
}

async fn read_capped_text(
    resp: reqwest::Response,
    max_bytes: usize,
    label: &str,
) -> CoreResult<String> {
    if let Some(len) = resp.content_length()
        && len as usize > max_bytes
    {
        return Err(CoreError::Solver(format!(
            "{label} exceeds max size ({len} > {max_bytes})"
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| CoreError::Solver(redact_secrets(&format!("read {label}: {e}"))))?;
    if bytes.len() > max_bytes {
        return Err(CoreError::Solver(format!(
            "{label} exceeds max size ({} > {max_bytes})",
            bytes.len()
        )));
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|e| CoreError::Solver(format!("{label} invalid utf-8: {e}")))
}
