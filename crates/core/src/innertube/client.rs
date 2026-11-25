//! InnerTube HTTP Client (`MusicProvider` implementation) with:
//! - Centralized `clients.rs` config table
//! - Persistent `X-Goog-Visitor-Id` (`visitorData`) in SQLite
//! - Strict per-client auth isolation (`WEB_REMIX` / `VISIONOS` vs `ANDROID_VR` AnonymousOnly)
//! - Playback resolution waterfall: `VISIONOS -> ANDROID_VR -> WEB_REMIX + rquickjs`
//! - Stale-While-Revalidate (SWR) SQLite caching with self-healing on corrupt JSON
//! - Graceful `401` cookie-rotation / session revocation recovery & `429` handling

use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use resonance_contract::{
    AuthError, AuthSession, AuthStore, CacheStore, CoreError, CoreResult, HomeSection,
    LyricsDocument, MusicProvider, Playlist, PoTokenError, PoTokenProvider, PoTokenRequest,
    PoTokenScope, ProviderError, SearchResults, StreamInfo, Track,
};
use serde_json::{Value, json};

use crate::innertube::auth::{apply_cookie_rotation, build_auth_headers};
use crate::innertube::clients::{AuthMode, ClientConfig, WEB_REMIX};
use crate::innertube::formats::TargetPlatform;
use crate::innertube::matrix::{AccountTier, ClientMatrix};
use crate::innertube::parser::{
    extract_lyrics_browse_id, extract_visitor_data, finalize_stream_info,
    parse_collection_response, parse_home_response, parse_next_queue_response,
    parse_player_response_for_platform, parse_search_response, parse_ytm_lyrics_response,
};
use crate::innertube::provider_adapter::PlaybackResolver;
use crate::potoken::attach_pot_param;
use crate::solver::SharedSolver;
use crate::unix_secs;

const TTL_HOME_SECS: u64 = 15 * 60;
const TTL_SEARCH_SECS: u64 = 5 * 60;
const TTL_COLLECTION_SECS: u64 = 60 * 60;
const TTL_STREAM_SECS: u64 = 4 * 3600;
const ANON_CONSENT_COOKIE: &str = "SOCS=CAISAiAD";

pub struct InnerTubeClient {
    http: reqwest::Client,
    cache: Arc<dyn CacheStore>,
    auth_store: Arc<dyn AuthStore>,
    solver: SharedSolver,
    session: RwLock<Option<AuthSession>>,
    visitor_data: RwLock<Option<String>>,
    hl: String,
    gl: String,
    /// Optional fallback JS solver script (e.g. `yt-dlp-ejs` bundle or test harness script).
    player_solver_js: RwLock<Option<(String, String)>>,
    /// Optional base URL override for local HTTP fault-injection tests.
    api_base_override: RwLock<Option<String>>,
    /// Remote-updatable data-driven client viability matrix (`client_matrix.json`).
    client_matrix: RwLock<ClientMatrix>,
    /// Native decoder target platform governing audio format preference.
    target_platform: TargetPlatform,
    /// Optional GVS PO token for free/anonymous web playback (bypassed for Premium subscribers).
    gvs_po_token: RwLock<Option<String>>,
    /// Dependency-injected PO token provider (e.g. `CachingPoTokenProvider<SidecarPoTokenProvider>`).
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
            visitor_data: RwLock::new(saved_visitor),
            hl: "en".to_owned(),
            gl: "US".to_owned(),
            player_solver_js: RwLock::new(None),
            api_base_override: RwLock::new(None),
            client_matrix: RwLock::new(matrix),
            target_platform: TargetPlatform::current(),
            gvs_po_token: RwLock::new(None),
            po_token_provider: RwLock::new(None),
            is_premium_account: RwLock::new(false),
        })
    }

    pub fn update_client_matrix(&self, matrix: ClientMatrix) {
        *self.client_matrix.write().unwrap() = matrix;
    }

    pub fn set_gvs_po_token(&self, token: Option<String>) {
        *self.gvs_po_token.write().unwrap() = token;
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

    pub fn sign_in(&self, session: AuthSession) -> CoreResult<()> {
        self.auth_store.save_session(&session)?;
        if let Some(vd) = &session.visitor_data {
            self.persist_visitor_data(vd);
        }
        *self.session.write().unwrap() = Some(session);
        Ok(())
    }

    pub fn sign_out(&self) -> CoreResult<()> {
        self.auth_store.clear_session()?;
        *self.session.write().unwrap() = None;
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

        let active_session = self.current_session();
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

        // Capture any Set-Cookie rotation headers for signed-in sessions on cookie-capable clients.
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
                let _ = self.auth_store.save_session(&updated);
                *self.session.write().unwrap() = Some(updated);
            }
        }

        if status.as_u16() == 401 {
            // Graceful 401 cookie-rotation / revocation recovery: clear stale session and report AuthError::Revoked.
            if active_session.is_some() && client.allows_browser_cookies() {
                let _ = self.sign_out();
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
                let req = PoTokenRequest {
                    client_name: client.client_name.to_owned(),
                    scope: PoTokenScope::Gvs,
                    video_id: video_id.to_owned(),
                    visitor_data: self.current_visitor_data(),
                    signed_in: self.current_session().is_some(),
                };
                Some(provider.mint_token(&req).await?.value)
            } else if let Some(legacy) = self.gvs_po_token.read().unwrap().clone() {
                Some(legacy)
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
                self.solver.solve_cipher_url(&hash, &js, &cipher_like).await?
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

    /// Reads an SWR cache entry, self-healing (evicting) on corrupt JSON.
    fn read_swr<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<(T, bool)> {
        let now = unix_secs();
        match self.cache.get_metadata(key, now) {
            Ok(Some((raw, is_stale))) => match serde_json::from_str::<T>(&raw) {
                Ok(val) => Some((val, is_stale)),
                Err(e) => {
                    log::warn!("corrupt metadata cache entry {key} ({e}); self-healing by evicting");
                    let _ = self.cache.evict_metadata(key);
                    None
                }
            },
            _ => None,
        }
    }

    fn write_swr<T: serde::Serialize>(&self, key: &str, value: &T, ttl_secs: u64) {
        if let Ok(json_str) = serde_json::to_string(value) {
            let _ = self.cache.put_metadata(key, &json_str, ttl_secs, unix_secs());
        }
    }
}

#[async_trait]
impl MusicProvider for InnerTubeClient {
    async fn search(&self, query: &str) -> CoreResult<SearchResults> {
        let cache_key = format!("search:{}", query.trim().to_lowercase());
        if let Some((cached, false)) = self.read_swr::<SearchResults>(&cache_key) {
            return Ok(cached);
        }

        let payload = json!({
            "query": query,
            // Songs filter parameter on YouTube Music WEB_REMIX.
            "params": "EgWKAQIIAWoKEAMQBBAJEAoQBQ%3D%3D",
        });

        match self.post_innertube(&WEB_REMIX, "search", payload).await {
            Ok(resp) => {
                let parsed = parse_search_response(query, &resp)?;
                self.write_swr(&cache_key, &parsed, TTL_SEARCH_SECS);
                Ok(parsed)
            }
            Err(err) => {
                if let Some((stale, _)) = self.read_swr::<SearchResults>(&cache_key) {
                    Ok(stale)
                } else {
                    Err(err)
                }
            }
        }
    }

    async fn home(&self) -> CoreResult<Vec<HomeSection>> {
        const KEY: &str = "home:default";
        if let Some((cached, false)) = self.read_swr::<Vec<HomeSection>>(KEY) {
            return Ok(cached);
        }
        let payload = json!({ "browseId": "FEmusic_home" });
        match self.post_innertube(&WEB_REMIX, "browse", payload).await {
            Ok(resp) => {
                let sections = parse_home_response(&resp)?;
                self.write_swr(KEY, &sections, TTL_HOME_SECS);
                Ok(sections)
            }
            Err(err) => {
                if let Some((stale, _)) = self.read_swr::<Vec<HomeSection>>(KEY) {
                    Ok(stale)
                } else {
                    Err(err)
                }
            }
        }
    }

    async fn browse_collection(&self, browse_id: &str) -> CoreResult<Playlist> {
        let key = format!("collection:{browse_id}");
        if let Some((cached, false)) = self.read_swr::<Playlist>(&key) {
            return Ok(cached);
        }
        let payload = json!({ "browseId": browse_id });
        match self.post_innertube(&WEB_REMIX, "browse", payload).await {
            Ok(resp) => {
                let playlist = parse_collection_response(browse_id, &resp)?;
                self.write_swr(&key, &playlist, TTL_COLLECTION_SECS);
                Ok(playlist)
            }
            Err(err) => {
                if let Some((stale, _)) = self.read_swr::<Playlist>(&key) {
                    Ok(stale)
                } else {
                    Err(err)
                }
            }
        }
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
        });
        let resp = self.post_innertube(&WEB_REMIX, "next", payload).await?;
        parse_next_queue_response(&resp)
    }

    /// Data-driven playback resolution chain loaded from `client_matrix.json`,
    /// gated by PO-token policy (`AccountTier`) and native decoder capabilities (`TargetPlatform`).
    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo> {
        let cache_key = format!("stream:{video_id}");
        if let Some((cached, false)) = self.read_swr::<StreamInfo>(&cache_key)
            && cached.expires_at_unix_secs > unix_secs() + 60
        {
            return Ok(cached);
        }

        let tier = self.account_tier();
        let has_gvs_po = self.po_token_provider.read().unwrap().is_some()
            || self.gvs_po_token.read().unwrap().is_some();
        let chain = self
            .client_matrix
            .read()
            .unwrap()
            .playback_chain(tier, has_gvs_po, false, false);

        let mut last_err = None;
        for client in chain {
            match self.resolve_with_client(client, video_id).await {
                Ok(stream) => {
                    self.write_swr(&cache_key, &stream, TTL_STREAM_SECS);
                    return Ok(stream);
                }
                Err(e) => {
                    log::warn!(
                        "stream resolve via {} failed for {video_id}: {e}",
                        client.client_name
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
