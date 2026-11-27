//! InnerTube HTTP Client (`MusicProvider` implementation) with:
//! - Centralized `clients.rs` config table
//! - Persistent `X-Goog-Visitor-Id` (`visitorData`) in SQLite
//! - Strict per-client auth isolation (`WEB_REMIX` / `VISIONOS` vs `ANDROID_VR` AnonymousOnly)
//! - Playback resolution waterfall: `VISIONOS -> ANDROID_VR -> WEB_REMIX + rquickjs`
//! - Stream metadata SQLite caching with self-healing on corrupt JSON (catalog SWR lives in `HybridMusicProvider`)
//! - Graceful `401` cookie-rotation / session revocation recovery & `429` handling

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use resonance_contract::{
    AuthError, AuthSession, AuthStore, CacheStore, CoreError, CoreResult, HomeSection,
    LyricsDocument, MusicProvider, Playlist, PoTokenDiagnostics, PoTokenError, PoTokenHealthStatus,
    PoTokenProvider, PoTokenRequest, PoTokenScope, ProviderError, SearchResults, StreamInfo, Track,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::innertube::auth::{apply_cookie_rotation, build_auth_headers};
use crate::innertube::clients::{AuthMode, ClientConfig, WEB_REMIX};
use crate::innertube::formats::TargetPlatform;
use crate::innertube::matrix::{
    AccountTier, CircuitBreakerConfig, ClientCircuitBreaker, ClientHealthSnapshot, ClientMatrix,
};
use crate::innertube::parser::{
    extract_home_continuation, extract_lyrics_browse_id, extract_search_plays,
    extract_visitor_data, finalize_stream_info, parse_collection_response, parse_home_response,
    parse_next_queue_response, parse_player_response_for_platform, parse_search_response,
    parse_ytm_lyrics_response,
};
use crate::innertube::provider_adapter::PlaybackResolver;
use crate::potoken::attach_pot_param;
use crate::solver::SharedSolver;
use crate::unix_secs;

const TTL_STREAM_SECS: u64 = 4 * 3600;
const ANON_CONSENT_COOKIE: &str = "SOCS=CAISAiAD";

pub struct InnerTubeClient {
    http: reqwest::Client,
    cache: Arc<dyn CacheStore>,
    auth_store: Arc<dyn AuthStore>,
    solver: SharedSolver,
    session: RwLock<Option<AuthSession>>,
    /// Monotonic epoch bumped on `sign_in`/`sign_out` so in-flight `Set-Cookie` rotations
    /// or stale `401`s from an earlier session can never resurrect or clobber newer auth state.
    session_epoch: AtomicU64,
    visitor_data: RwLock<Option<String>>,
    hl: String,
    gl: String,
    /// Optional fallback JS solver script (e.g. `yt-dlp-ejs` bundle or test harness script).
    player_solver_js: RwLock<Option<(String, String)>>,
    /// Optional base URL override for local HTTP fault-injection tests.
    api_base_override: RwLock<Option<String>>,
    /// Remote-updatable data-driven client viability matrix (`client_matrix.json`).
    client_matrix: RwLock<ClientMatrix>,
    /// Adaptive per-client circuit breaker tracking rolling 403/429 rates and latency.
    circuit_breaker: Arc<ClientCircuitBreaker>,
    /// Native decoder target platform governing audio format preference.
    target_platform: TargetPlatform,
    /// Dependency-injected PO token provider (e.g. `CachingPoTokenProvider<ChainedPoTokenProvider>`).
    po_token_provider: RwLock<Option<Arc<dyn PoTokenProvider>>>,
    /// Whether the active signed-in session has YouTube Music Premium (bypasses GVS PO token & unlocks 256 kbps).
    is_premium_account: RwLock<bool>,
}

impl InnerTubeClient {
    pub fn new(
        cache: Arc<dyn CacheStore>,
        auth_store: Arc<dyn AuthStore>,
        solver: SharedSolver,
    ) -> CoreResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .gzip(true)
            .build()
            .map_err(|e| CoreError::Provider(ProviderError::Network(e.to_string())))?;

        let saved_visitor = cache.get_visitor_data().unwrap_or(None);
        let saved_session = auth_store.load_session().unwrap_or(None);
        let matrix = ClientMatrix::embedded_default()?;

        Ok(Self {
            http,
            cache,
            auth_store,
            solver,
            session: RwLock::new(saved_session),
            session_epoch: AtomicU64::new(0),
            visitor_data: RwLock::new(saved_visitor),
            hl: "en".to_owned(),
            gl: "US".to_owned(),
            player_solver_js: RwLock::new(None),
            api_base_override: RwLock::new(None),
            client_matrix: RwLock::new(matrix),
            circuit_breaker: Arc::new(ClientCircuitBreaker::new(CircuitBreakerConfig::default())),
            target_platform: TargetPlatform::current(),
            po_token_provider: RwLock::new(None),
            is_premium_account: RwLock::new(false),
        })
    }

    pub fn with_circuit_breaker(mut self, breaker: Arc<ClientCircuitBreaker>) -> Self {
        self.circuit_breaker = breaker;
        self
    }

    pub fn circuit_breaker(&self) -> &Arc<ClientCircuitBreaker> {
        &self.circuit_breaker
    }

    pub fn client_health_snapshot(
        &self,
        kind: crate::innertube::clients::ClientKind,
    ) -> ClientHealthSnapshot {
        self.circuit_breaker.health_snapshot(kind)
    }

    pub fn po_token_diagnostics(&self) -> PoTokenDiagnostics {
        if let Some(provider) = self.po_token_provider.read().unwrap().as_ref() {
            provider.diagnostics()
        } else {
            PoTokenDiagnostics {
                status: PoTokenHealthStatus::Unconfigured,
                active_provider: "none".to_owned(),
                detail: "No PO token provider configured — fallback matrix clients active"
                    .to_owned(),
                degraded_fallback_active: true,
                last_checked_unix_secs: unix_secs(),
            }
        }
    }

    pub fn update_client_matrix(&self, matrix: ClientMatrix) {
        *self.client_matrix.write().unwrap() = matrix;
    }

    pub fn with_po_token_provider(self, provider: Arc<dyn PoTokenProvider>) -> Self {
        *self.po_token_provider.write().unwrap() = Some(provider);
        self
    }

    pub fn set_po_token_provider(&self, provider: Option<Arc<dyn PoTokenProvider>>) {
        *self.po_token_provider.write().unwrap() = provider;
    }

    pub fn invalidate_po_token(&self) {
        if let Some(provider) = self.po_token_provider.read().unwrap().as_ref() {
            provider.invalidate(PoTokenScope::Gvs);
        }
    }

    pub fn set_premium_account(&self, premium: bool) {
        *self.is_premium_account.write().unwrap() = premium;
    }

    pub fn account_tier(&self) -> AccountTier {
        if self.current_session().is_none() {
            AccountTier::Anonymous
        } else if *self.is_premium_account.read().unwrap() {
            AccountTier::PremiumSignedIn
        } else {
            AccountTier::FreeSignedIn
        }
    }

    pub fn http_client(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn current_session(&self) -> Option<AuthSession> {
        self.session.read().unwrap().clone()
    }

    pub(crate) fn session_with_epoch(&self) -> (u64, Option<AuthSession>) {
        let guard = self.session.read().unwrap();
        (self.session_epoch.load(Ordering::SeqCst), guard.clone())
    }

    pub(crate) fn rotate_session_if_epoch(&self, expected_epoch: u64, updated: AuthSession) {
        let mut guard = self.session.write().unwrap();
        if self.session_epoch.load(Ordering::SeqCst) == expected_epoch && guard.is_some() {
            let _ = self.auth_store.save_session(&updated);
            *guard = Some(updated);
        }
    }

    pub(crate) fn sign_out_if_epoch(&self, expected_epoch: u64) -> CoreResult<()> {
        let mut guard = self.session.write().unwrap();
        if self.session_epoch.load(Ordering::SeqCst) == expected_epoch {
            self.session_epoch.fetch_add(1, Ordering::SeqCst);
            self.auth_store.clear_session()?;
            *guard = None;
        }
        Ok(())
    }

    pub fn sign_in(&self, session: AuthSession) -> CoreResult<()> {
        self.auth_store.save_session(&session)?;
        if let Some(vd) = &session.visitor_data {
            self.persist_visitor_data(vd);
        }
        let mut guard = self.session.write().unwrap();
        self.session_epoch.fetch_add(1, Ordering::SeqCst);
        *guard = Some(session);
        Ok(())
    }

    pub fn sign_out(&self) -> CoreResult<()> {
        let mut guard = self.session.write().unwrap();
        self.session_epoch.fetch_add(1, Ordering::SeqCst);
        self.auth_store.clear_session()?;
        *guard = None;
        Ok(())
    }

    pub fn set_player_solver(&self, player_hash: String, solver_js: String) {
        *self.player_solver_js.write().unwrap() = Some((player_hash, solver_js));
    }

    pub fn set_api_base_override(&self, base: Option<String>) {
        *self.api_base_override.write().unwrap() = base;
    }

    pub fn current_visitor_data(&self) -> Option<String> {
        self.visitor_data.read().unwrap().clone()
    }

    fn persist_visitor_data(&self, vd: &str) {
        if vd.is_empty() {
            return;
        }
        let mut guard = self.visitor_data.write().unwrap();
        if guard.as_deref() != Some(vd) {
            *guard = Some(vd.to_owned());
            let _ = self.cache.set_visitor_data(vd);
        }
    }

    async fn post_innertube(
        &self,
        client: &ClientConfig,
        endpoint: &str,
        mut payload: Value,
    ) -> CoreResult<Value> {
        let vd = self.current_visitor_data();
        let ctx = client.build_context(vd.as_deref(), &self.hl, &self.gl);
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("context".into(), ctx);
        }

        let base = self
            .api_base_override
            .read()
            .unwrap()
            .clone()
            .unwrap_or_else(|| client.api_base.to_owned());
        let url = format!("{base}/{endpoint}?prettyPrint=false");
        let mut req = self
            .http
            .post(&url)
            .header("User-Agent", client.user_agent)
            .header("Origin", client.origin)
            .header("X-YouTube-Client-Name", client.client_id)
            .header("X-YouTube-Client-Version", client.client_version);

        if let Some(v) = vd.as_deref().filter(|s| !s.is_empty()) {
            req = req.header("X-Goog-Visitor-Id", v);
        }

        let (req_epoch, active_session) = self.session_with_epoch();
        if client.allows_browser_cookies() {
            if let Some(sess) = &active_session {
                let auth_headers = build_auth_headers(client, sess, unix_secs())?;
                for (k, v) in auth_headers {
                    req = req.header(k, v);
                }
            } else {
                req = req.header("Cookie", ANON_CONSENT_COOKIE);
            }
        }
        // Note: For AnonymousOnly clients (`ANDROID_VR`), NO cookies or Authorization headers are ever attached.

        let resp = req
            .json(&payload)
            .send()
            .await
            .map_err(|e| CoreError::Provider(ProviderError::Network(e.to_string())))?;

        let status = resp.status();

        // Capture any Set-Cookie rotation headers for signed-in sessions on cookie-capable clients,
        // fenced by `req_epoch` so an in-flight request can never resurrect a signed-out session.
        if client.allows_browser_cookies()
            && let Some(sess) = &active_session
        {
            let set_cookies: Vec<String> = resp
                .headers()
                .get_all(reqwest::header::SET_COOKIE)
                .iter()
                .filter_map(|h| h.to_str().ok().map(str::to_owned))
                .collect();
            if !set_cookies.is_empty()
                && let Some(updated) =
                    apply_cookie_rotation(sess, set_cookies.iter().map(String::as_str))
            {
                self.rotate_session_if_epoch(req_epoch, updated);
            }
        }

        if status.as_u16() == 401 {
            // Graceful 401 cookie-rotation / revocation recovery: clear stale session and report AuthError::Revoked.
            if active_session.is_some() && client.allows_browser_cookies() {
                let _ = self.sign_out_if_epoch(req_epoch);
                return Err(CoreError::Auth(AuthError::Revoked));
            }
        }
        if status.as_u16() == 429 {
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());
            return Err(CoreError::Provider(ProviderError::RateLimited {
                retry_after_secs: retry_after,
            }));
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(CoreError::Provider(ProviderError::Http {
                status: status.as_u16(),
                message: text.chars().take(240).collect(),
            }));
        }

        let json_val: Value = resp
            .json()
            .await
            .map_err(|e| CoreError::Provider(ProviderError::Parse(e.to_string())))?;

        if let Some(new_vd) = extract_visitor_data(&json_val) {
            self.persist_visitor_data(&new_vd);
        }

        Ok(json_val)
    }

    pub(crate) fn web_remix_base_url(&self) -> String {
        self.api_base_override
            .read()
            .unwrap()
            .clone()
            .unwrap_or_else(|| WEB_REMIX.api_base.to_owned())
    }

    pub(crate) async fn post_authenticated_web_remix(
        &self,
        endpoint: &str,
        payload: Value,
    ) -> CoreResult<Value> {
        if self.current_session().is_none() {
            return Err(CoreError::Auth(AuthError::NotSignedIn));
        }
        self.post_innertube(&WEB_REMIX, endpoint, payload).await
    }

    pub(crate) fn evict_swr_cache(&self, key: &str) {
        let _ = self.cache.evict_metadata(key);
    }

    async fn resolve_with_client(
        &self,
        client: &ClientConfig,
        video_id: &str,
    ) -> CoreResult<StreamInfo> {
        let tier = self.account_tier();
        let entry = self
            .client_matrix
            .read()
            .unwrap()
            .entry_for(client.kind)
            .cloned();

        // Hard guard: AnonymousOnly clients (`ANDROID_VR`, `IOS`) NEVER receive cookies,
        // `SAPISIDHASH`, `serviceIntegrityDimensions`, or `&pot=`.
        let needs_pot = if matches!(client.auth_mode, AuthMode::AnonymousOnly)
            || !client.allows_browser_cookies()
        {
            false
        } else {
            entry.is_some_and(|e| e.po_token_policy.requires_gvs_po_token(tier))
        };

        let pot_value = if needs_pot {
            let provider = self.po_token_provider.read().unwrap().clone();
            if let Some(provider) = provider {
                let session_hash = self.current_session().map(|sess| {
                    let mut h = Sha256::new();
                    h.update(sess.cookie_header.as_bytes());
                    hex::encode(h.finalize())[..16].to_owned()
                });
                let req = PoTokenRequest {
                    client_name: client.client_name.to_owned(),
                    scope: PoTokenScope::Gvs,
                    video_id: video_id.to_owned(),
                    visitor_data: self.current_visitor_data(),
                    session_hash,
                    signed_in: self.current_session().is_some(),
                };
                Some(provider.mint_token(&req).await?.value)
            } else {
                return Err(CoreError::PoToken(PoTokenError::Unavailable(
                    "no po token provider configured".to_owned(),
                )));
            }
        } else {
            None
        };

        let mut payload = json!({
            "videoId": video_id,
            "contentCheckOk": true,
            "racyCheckOk": true,
        });
        if let Some(ref pot) = pot_value
            && let Some(obj) = payload.as_object_mut()
        {
            obj.insert(
                "serviceIntegrityDimensions".into(),
                json!({ "poToken": pot }),
            );
        }

        let resp = self.post_innertube(client, "player", payload).await?;
        let now = unix_secs();
        let raw = parse_player_response_for_platform(
            video_id,
            client.client_name,
            &resp,
            now,
            self.target_platform,
            tier,
        )?;

        let url = if let Some(cipher) = raw.signature_cipher.clone() {
            let solver_bundle = self.player_solver_js.read().unwrap().clone();
            let Some((hash, js)) = solver_bundle else {
                return Err(CoreError::Solver(format!(
                    "ciphered stream from {} requires updatable yt-dlp-ejs solver bundle",
                    client.client_name
                )));
            };
            self.solver.solve_cipher_url(&hash, &js, &cipher).await?
        } else if let Some(plain) = raw.url.clone() {
            // Proactively solve `n`-parameter via EJS when client requires JS solver (e.g. signed-in WEB_REMIX).
            let solver_bundle = if client.needs_js_solver && plain.contains("n=") {
                self.player_solver_js.read().unwrap().clone()
            } else {
                None
            };
            if let Some((hash, js)) = solver_bundle {
                let cipher_like = format!(
                    "url={}",
                    url::form_urlencoded::byte_serialize(plain.as_bytes()).collect::<String>()
                );
                self.solver
                    .solve_cipher_url(&hash, &js, &cipher_like)
                    .await?
            } else {
                plain
            }
        } else {
            return Err(CoreError::Provider(ProviderError::NoStream {
                video_id: video_id.to_owned(),
            }));
        };

        let final_url = if let Some(ref pot) = pot_value {
            attach_pot_param(&url, pot)?
        } else {
            url
        };

        Ok(finalize_stream_info(
            video_id,
            client.client_name,
            raw,
            final_url,
            now,
        ))
    }

    fn read_cache_entry<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<(T, bool)> {
        let now = unix_secs();
        match self.cache.get_metadata(key, now) {
            Ok(Some((raw, is_stale))) => match serde_json::from_str::<T>(&raw) {
                Ok(val) => Some((val, is_stale)),
                Err(e) => {
                    log::warn!(
                        "corrupt metadata cache entry {key} ({e}); self-healing by evicting"
                    );
                    let _ = self.cache.evict_metadata(key);
                    None
                }
            },
            _ => None,
        }
    }

    fn write_cache_entry<T: serde::Serialize>(&self, key: &str, value: &T, ttl_secs: u64) {
        if let Ok(json_str) = serde_json::to_string(value) {
            let _ = self
                .cache
                .put_metadata(key, &json_str, ttl_secs, unix_secs());
        }
    }

    /// Searches YouTube Music with an optional InnerTube filter `params` token
    /// (e.g. `None` for All, `"EgWKAQIIAWoKEAMQBBAJEAoQBQ%3D%3D"` for Songs,
    /// `"EgWKAQIYAWoKEAMQBBAJEAoQBQ%3D%3D"` for Albums).
    pub async fn search_with_params(
        &self,
        query: &str,
        params: Option<&str>,
    ) -> CoreResult<SearchResults> {
        let (res, _) = self.search_with_params_and_plays(query, params).await?;
        Ok(res)
    }

    /// Searches YouTube Music and also returns a `video_id -> plays_text` map when available.
    pub async fn search_with_params_and_plays(
        &self,
        query: &str,
        params: Option<&str>,
    ) -> CoreResult<(SearchResults, HashMap<String, String>)> {
        let trimmed = query.trim();
        let mut payload = json!({ "query": trimmed });
        if let Some(p) = params
            && let Some(obj) = payload.as_object_mut()
        {
            obj.insert("params".into(), json!(p));
        }

        let resp = self.post_innertube(&WEB_REMIX, "search", payload).await?;
        let parsed = parse_search_response(trimmed, &resp)?;
        let plays = extract_search_plays(&resp);
        Ok((parsed, plays))
    }

    /// Fetches `FEmusic_explore` (New albums & singles, Trending, Moods & genres, New music videos).
    pub async fn explore(&self) -> CoreResult<Vec<HomeSection>> {
        let payload = json!({ "browseId": "FEmusic_explore" });
        let resp = self.post_innertube(&WEB_REMIX, "browse", payload).await?;
        parse_home_response(&resp)
    }
}

#[async_trait]
impl MusicProvider for InnerTubeClient {
    async fn search(&self, query: &str) -> CoreResult<SearchResults> {
        self.search_with_params(query, None).await
    }

    async fn home(&self) -> CoreResult<Vec<HomeSection>> {
        let payload = json!({ "browseId": "FEmusic_home" });
        let resp = self.post_innertube(&WEB_REMIX, "browse", payload).await?;
        let mut sections = parse_home_response(&resp)?;
        let mut cont = extract_home_continuation(&resp);
        let mut page = 0usize;
        while sections.len() < 5
            && page < 2
            && let Some(token) = cont.take()
        {
            page += 1;
            let Ok(next_resp) = self
                .post_innertube(&WEB_REMIX, "browse", json!({ "continuation": token }))
                .await
            else {
                break;
            };
            if let Ok(mut more) = parse_home_response(&next_resp) {
                for (i, sec) in more.iter_mut().enumerate() {
                    sec.id = format!("home-c{page}-{i}");
                }
                sections.extend(more);
            }
            cont = extract_home_continuation(&next_resp);
        }
        Ok(sections)
    }

    async fn browse_collection(&self, browse_id: &str) -> CoreResult<Playlist> {
        let payload = json!({ "browseId": browse_id });
        let resp = self.post_innertube(&WEB_REMIX, "browse", payload).await?;
        parse_collection_response(browse_id, &resp)
    }

    async fn liked_songs(&self) -> CoreResult<Vec<Track>> {
        if self.current_session().is_none() {
            return Ok(Vec::new());
        }
        let playlist = self.browse_collection("FEmusic_liked_videos").await?;
        Ok(playlist.tracks)
    }

    async fn autoplay_next(&self, video_id: &str) -> CoreResult<Vec<Track>> {
        let payload = json!({
            "videoId": video_id,
            "playlistId": format!("RDAMVM{video_id}"),
            "isAudioOnly": true,
            "enablePersistentPlaylistPanel": true,
            "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
            "watchEndpointMusicSupportedConfigs": {
                "watchEndpointMusicConfig": {
                    "hasPersistentPlaylistPanel": true,
                    "musicVideoType": "MUSIC_VIDEO_TYPE_ATV"
                }
            }
        });
        let resp = self.post_innertube(&WEB_REMIX, "next", payload).await?;
        parse_next_queue_response(&resp)
    }

    /// Data-driven playback resolution chain loaded from `client_matrix.json`,
    /// gated by PO-token policy (`AccountTier`) and native decoder capabilities (`TargetPlatform`).
    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo> {
        let cache_key = format!("stream:{video_id}");
        if let Some((cached, false)) = self.read_cache_entry::<StreamInfo>(&cache_key)
            && cached.expires_at_unix_secs > unix_secs() + 60
        {
            return Ok(cached);
        }

        let tier = self.account_tier();
        let has_gvs_po = self.po_token_provider.read().unwrap().is_some();
        let chain = self
            .client_matrix
            .read()
            .unwrap()
            .playback_chain(tier, has_gvs_po, false, false);

        struct ProbeGuard<'a> {
            breaker: &'a ClientCircuitBreaker,
            kind: crate::innertube::clients::ClientKind,
            active: bool,
        }
        impl Drop for ProbeGuard<'_> {
            fn drop(&mut self) {
                if self.active {
                    self.breaker.release_probe(self.kind);
                }
            }
        }

        let mut last_err = None;
        for client in chain {
            if !self.circuit_breaker.allow_request(client.kind) {
                log::debug!(
                    "skipping client {} for {video_id}: circuit breaker OPEN",
                    client.client_name
                );
                continue;
            }
            let mut probe_guard = ProbeGuard {
                breaker: &self.circuit_breaker,
                kind: client.kind,
                active: true,
            };
            let started = Instant::now();
            match self.resolve_with_client(client, video_id).await {
                Ok(stream) => {
                    probe_guard.active = false;
                    self.circuit_breaker
                        .record_success(client.kind, started.elapsed().as_millis() as u64);
                    self.write_cache_entry(&cache_key, &stream, TTL_STREAM_SECS);
                    return Ok(stream);
                }
                Err(e) => {
                    let latency_ms = started.elapsed().as_millis() as u64;
                    let http_status = match &e {
                        CoreError::Provider(ProviderError::Http { status, .. }) => Some(*status),
                        CoreError::Provider(ProviderError::RateLimited { .. }) => Some(429),
                        _ => None,
                    };
                    if matches!(
                        &e,
                        CoreError::Provider(ProviderError::Http {
                            status: 403 | 429 | 500..=599,
                            ..
                        }) | CoreError::Provider(ProviderError::RateLimited { .. })
                            | CoreError::Provider(ProviderError::Network(_))
                    ) {
                        probe_guard.active = false;
                        self.circuit_breaker
                            .record_failure(client.kind, http_status, latency_ms);
                    }
                    // Otherwise `probe_guard` drops here and releases any HalfOpen probe slot.
                    log::warn!(
                        "stream resolve via {} failed for {video_id}: {}",
                        client.client_name,
                        crate::innertube::auth::redact_secrets(&e.to_string())
                    );
                    last_err = Some(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            CoreError::Provider(ProviderError::NoStream {
                video_id: video_id.to_owned(),
            })
        }))
    }

    async fn ytm_lyrics(&self, video_id: &str) -> CoreResult<Option<LyricsDocument>> {
        let next_resp = self
            .post_innertube(&WEB_REMIX, "next", json!({ "videoId": video_id }))
            .await?;
        let Some(browse_id) = extract_lyrics_browse_id(&next_resp) else {
            return Ok(None);
        };
        let browse_resp = self
            .post_innertube(&WEB_REMIX, "browse", json!({ "browseId": browse_id }))
            .await?;
        Ok(parse_ytm_lyrics_response(video_id, &browse_resp))
    }
}

#[async_trait]
impl PlaybackResolver for InnerTubeClient {
    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo> {
        <Self as MusicProvider>::resolve_stream(self, video_id).await
    }
}
