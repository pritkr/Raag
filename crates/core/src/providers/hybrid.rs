//! `HybridMusicProvider` — Single unified catalog, stream, and mutation facade composing:
//! 1. Browse/Catalog failover: `YtmapiBrowseAdapter` -> `RustyPipeBrowseAdapter` -> `InnerTubeClient`
//! 2. Playback failover: `InnerTubeClient` (`ClientMatrix` + `rquickjs` + PO-Token) -> `RustyPipePlaybackAdapter`
//! 3. Single Stale-While-Revalidate (SWR) metadata cache layer (disables inner client catalog SWR)
//! 4. Account, PO-Token, and `InnerTubeMutations` delegation

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use async_trait::async_trait;
use resonance_contract::{
    AlbumDetail, ArtistDetail, AuthSession, CacheStore, CatalogExtProvider, CoreError, CoreResult,
    ExplorePage, HomeSection, InnerTubeMutations, LibraryCollectionRef, LibrarySnapshot,
    LikeAction, LyricsDocument, MusicProvider, MutationError, MutationReceipt, Playlist,
    PlaylistEditOp, PoTokenDiagnostics, PoTokenHealthStatus, PoTokenProvider, ProviderError,
    SearchCategory, SearchResults, StreamInfo, Track, WatchtimeReport,
};
use serde::{Deserialize, Serialize};

use crate::innertube::client::InnerTubeClient;
use crate::innertube::matrix::ClientMatrix;
use crate::innertube::provider_adapter::{BrowseDelegate, PlaybackResolver};
use crate::providers::mappers::{deduplicate_playlists, deduplicate_tracks};
use crate::unix_secs;

#[cfg(feature = "provider-rustypipe")]
use crate::providers::rustypipe_adapter::{RustyPipeBrowseAdapter, RustyPipePlaybackAdapter};
#[cfg(feature = "provider-ytmapi")]
use crate::providers::ytmapi_adapter::YtmapiBrowseAdapter;

const TTL_HYBRID_HOME_SECS: u64 = 15 * 60;
const TTL_HYBRID_SEARCH_SECS: u64 = 5 * 60;
const TTL_HYBRID_COLLECTION_SECS: u64 = 60 * 60;
const CIRCUIT_FAILURE_THRESHOLD: u32 = 3;
const CIRCUIT_COOLDOWN_SECS: u64 = 120;

#[derive(Default)]
struct PrimaryCircuitBreaker {
    consecutive_failures: AtomicU32,
    cooldown_until_unix_secs: AtomicU64,
}

impl PrimaryCircuitBreaker {
    fn is_open(&self) -> bool {
        let until = self.cooldown_until_unix_secs.load(Ordering::Acquire);
        if until == 0 {
            return false;
        }
        if unix_secs() >= until {
            self.cooldown_until_unix_secs.store(0, Ordering::Release);
            self.consecutive_failures.store(0, Ordering::Release);
            return false;
        }
        true
    }

    fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Release);
        self.cooldown_until_unix_secs.store(0, Ordering::Release);
    }

    fn record_failure(&self, err: &CoreError) {
        // Auth errors must never trip the parser/provider circuit breaker.
        if matches!(err, CoreError::Auth(_)) {
            return;
        }
        let failures = self.consecutive_failures.fetch_add(1, Ordering::AcqRel) + 1;
        if failures >= CIRCUIT_FAILURE_THRESHOLD {
            self.cooldown_until_unix_secs.store(
                unix_secs().saturating_add(CIRCUIT_COOLDOWN_SECS),
                Ordering::Release,
            );
        }
    }
}

/// Single unified music provider orchestrating `ytmapi-rs`, `rustypipe`, and `InnerTubeClient`
/// behind one SWR cache layer and one facade API.
pub struct HybridMusicProvider {
    inner: Option<Arc<InnerTubeClient>>,
    cache: Option<Arc<dyn CacheStore>>,
    #[cfg(feature = "provider-ytmapi")]
    ytmapi: Option<Arc<YtmapiBrowseAdapter>>,
    #[cfg(feature = "provider-rustypipe")]
    rp_browse: Option<Arc<RustyPipeBrowseAdapter>>,
    primary_browse: Option<Arc<dyn BrowseDelegate>>,
    secondary_browse: Option<Arc<dyn BrowseDelegate>>,
    primary_playback: Option<Arc<dyn PlaybackResolver>>,
    secondary_playback: Option<Arc<dyn PlaybackResolver>>,
    primary_breaker: PrimaryCircuitBreaker,
}

impl HybridMusicProvider {
    pub fn new(inner: Arc<InnerTubeClient>, cache: Arc<dyn CacheStore>) -> Self {
        // HybridMusicProvider owns the single catalog SWR layer; prevent double caching in InnerTubeClient.
        inner.set_swr_cache_enabled(false);

        #[cfg(feature = "provider-ytmapi")]
        let ytmapi = Some(Arc::new(YtmapiBrowseAdapter::new(Arc::clone(&cache))));

        #[cfg(feature = "provider-rustypipe")]
        let (rp_browse, secondary_playback) = {
            let browse = Arc::new(RustyPipeBrowseAdapter::new(None));
            let playback: Arc<dyn PlaybackResolver> =
                Arc::new(RustyPipePlaybackAdapter::new(browse.shared_client()));
            (Some(browse), Some(playback))
        };
        #[cfg(not(feature = "provider-rustypipe"))]
        let secondary_playback: Option<Arc<dyn PlaybackResolver>> = None;

        let primary_browse: Option<Arc<dyn BrowseDelegate>> = {
            #[cfg(feature = "provider-ytmapi")]
            {
                ytmapi
                    .as_ref()
                    .map(|a| Arc::clone(a) as Arc<dyn BrowseDelegate>)
            }
            #[cfg(not(feature = "provider-ytmapi"))]
            {
                None
            }
        };

        let secondary_browse: Option<Arc<dyn BrowseDelegate>> = {
            #[cfg(feature = "provider-rustypipe")]
            {
                rp_browse
                    .as_ref()
                    .map(|a| Arc::clone(a) as Arc<dyn BrowseDelegate>)
            }
            #[cfg(not(feature = "provider-rustypipe"))]
            {
                None
            }
        };

        Self {
            primary_playback: Some(Arc::clone(&inner) as Arc<dyn PlaybackResolver>),
            inner: Some(inner),
            cache: Some(cache),
            #[cfg(feature = "provider-ytmapi")]
            ytmapi,
            #[cfg(feature = "provider-rustypipe")]
            rp_browse,
            primary_browse,
            secondary_browse,
            secondary_playback,
            primary_breaker: PrimaryCircuitBreaker::default(),
        }
    }

    /// Lightweight delegate-only composition (replaces legacy `CompositeMusicProvider`).
    pub fn from_delegates(
        browse: Arc<dyn BrowseDelegate>,
        playback: Arc<dyn PlaybackResolver>,
    ) -> Self {
        Self {
            inner: None,
            cache: None,
            #[cfg(feature = "provider-ytmapi")]
            ytmapi: None,
            #[cfg(feature = "provider-rustypipe")]
            rp_browse: None,
            primary_browse: Some(browse),
            secondary_browse: None,
            primary_playback: Some(playback),
            secondary_playback: None,
            primary_breaker: PrimaryCircuitBreaker::default(),
        }
    }

    /// Dependency-injection constructor for deterministic failover testing.
    pub fn with_delegates(
        inner: Arc<InnerTubeClient>,
        cache: Arc<dyn CacheStore>,
        primary_browse: Option<Arc<dyn BrowseDelegate>>,
        secondary_browse: Option<Arc<dyn BrowseDelegate>>,
        primary_playback: Option<Arc<dyn PlaybackResolver>>,
        secondary_playback: Option<Arc<dyn PlaybackResolver>>,
    ) -> Self {
        inner.set_swr_cache_enabled(false);
        Self {
            inner: Some(inner),
            cache: Some(cache),
            #[cfg(feature = "provider-ytmapi")]
            ytmapi: None,
            #[cfg(feature = "provider-rustypipe")]
            rp_browse: None,
            primary_browse,
            secondary_browse,
            primary_playback,
            secondary_playback,
            primary_breaker: PrimaryCircuitBreaker::default(),
        }
    }

    /// Returns the underlying `InnerTubeClient` when concrete protocol access is required.
    pub fn inner_client(&self) -> Option<&Arc<InnerTubeClient>> {
        self.inner.as_ref()
    }

    pub fn current_session(&self) -> Option<AuthSession> {
        self.inner.as_ref().and_then(|c| c.current_session())
    }

    pub fn current_visitor_data(&self) -> Option<String> {
        self.inner.as_ref().and_then(|c| c.current_visitor_data())
    }

    pub fn update_client_matrix(&self, matrix: ClientMatrix) {
        if let Some(inner) = &self.inner {
            inner.update_client_matrix(matrix);
        }
    }

    pub fn set_po_token_provider(&self, provider: Option<Arc<dyn PoTokenProvider>>) {
        if let Some(inner) = &self.inner {
            inner.set_po_token_provider(provider);
        }
    }

    pub fn set_premium_account(&self, premium: bool) {
        if let Some(inner) = &self.inner {
            inner.set_premium_account(premium);
        }
    }

    pub fn po_token_diagnostics(&self) -> PoTokenDiagnostics {
        if let Some(inner) = &self.inner {
            inner.po_token_diagnostics()
        } else {
            PoTokenDiagnostics {
                status: PoTokenHealthStatus::Unconfigured,
                active_provider: "none".to_owned(),
                detail: "Delegate-only provider has no InnerTube client".to_owned(),
                degraded_fallback_active: true,
                last_checked_unix_secs: unix_secs(),
            }
        }
    }

    /// Returns `true` when the primary browse circuit breaker is currently open.
    pub fn is_primary_circuit_open(&self) -> bool {
        self.primary_breaker.is_open()
    }

    /// Synchronizes `AuthSession` into `InnerTubeClient` and `YtmapiBrowseAdapter`.
    /// Note: `rustypipe` adapters intentionally NEVER receive browser cookies.
    pub async fn sync_session(&self, session: Option<&AuthSession>) -> CoreResult<()> {
        if let Some(inner) = &self.inner {
            if let Some(s) = session {
                inner.sign_in(s.clone())?;
            } else {
                inner.sign_out()?;
            }
        }
        #[cfg(feature = "provider-ytmapi")]
        if let Some(ytmapi) = &self.ytmapi {
            ytmapi.sync_session(session).await?;
        }
        Ok(())
    }

    /// Searches YouTube Music with optional InnerTube filter `params` and play-count map,
    /// backed by `HybridMusicProvider`'s single SWR cache layer.
    pub async fn search_with_params_and_plays(
        &self,
        query: &str,
        params: Option<&str>,
    ) -> CoreResult<(SearchResults, HashMap<String, String>)> {
        let trimmed = query.trim();
        let filter_tag = params.unwrap_or("all");
        let cache_key = format!("hybrid:search:{filter_tag}:{}", trimmed.to_lowercase());
        let plays_key = format!("hybrid:search_plays:{filter_tag}:{}", trimmed.to_lowercase());

        let (cached, is_stale) = self.read_swr::<SearchResults>(&cache_key);
        if let Some(fresh) = cached.as_ref().filter(|_| !is_stale) {
            let cached_plays = self
                .read_swr::<HashMap<String, String>>(&plays_key)
                .0
                .unwrap_or_default();
            return Ok((fresh.clone(), cached_plays));
        }

        if let Some(inner) = &self.inner {
            match inner.search_with_params_and_plays(trimmed, params).await {
                Ok((parsed, plays)) => {
                    if !parsed.tracks.is_empty() || !parsed.albums.is_empty() {
                        self.write_swr(&cache_key, &parsed, TTL_HYBRID_SEARCH_SECS);
                        if !plays.is_empty() {
                            self.write_swr(&plays_key, &plays, TTL_HYBRID_SEARCH_SECS);
                        }
                    }
                    return Ok((parsed, plays));
                }
                Err(err) => {
                    // Fail over to catalog delegates when unfiltered or if stale cache exists.
                    if params.is_none()
                        && let Ok(res) = self.search(trimmed).await
                    {
                        return Ok((res, HashMap::new()));
                    }
                    if let Some(stale) = cached {
                        let cached_plays = self
                            .read_swr::<HashMap<String, String>>(&plays_key)
                            .0
                            .unwrap_or_default();
                        return Ok((stale, cached_plays));
                    }
                    return Err(err);
                }
            }
        }

        let res = self.search(trimmed).await?;
        Ok((res, HashMap::new()))
    }

    /// Reads an SWR metadata entry, returning `(value, is_stale)`.
    fn read_swr<T: for<'de> Deserialize<'de>>(&self, key: &str) -> (Option<T>, bool) {
        let Some(cache) = &self.cache else {
            return (None, true);
        };
        let now = unix_secs();
        match cache.get_metadata(key, now) {
            Ok(Some((json, is_stale))) => match serde_json::from_str::<T>(&json) {
                Ok(val) => (Some(val), is_stale),
                Err(_) => {
                    let _ = cache.evict_metadata(key);
                    (None, true)
                }
            },
            _ => (None, true),
        }
    }

    fn write_swr<T: Serialize>(&self, key: &str, val: &T, ttl_secs: u64) {
        let Some(cache) = &self.cache else {
            return;
        };
        if let Ok(json) = serde_json::to_string(val) {
            let _ = cache.put_metadata(key, &json, ttl_secs, unix_secs());
        }
    }
}

#[async_trait]
impl MusicProvider for HybridMusicProvider {
    async fn search(&self, query: &str) -> CoreResult<SearchResults> {
        let cache_key = format!("hybrid:search:{}", query.trim());
        let (cached, is_stale) = self.read_swr::<SearchResults>(&cache_key);
        if let Some(fresh) = cached.as_ref().filter(|_| !is_stale) {
            return Ok(fresh.clone());
        }

        if !self.primary_breaker.is_open()
            && let Some(primary) = &self.primary_browse
        {
            match primary.search(query).await {
                Ok(res) => {
                    self.primary_breaker.record_success();
                    self.write_swr(&cache_key, &res, TTL_HYBRID_SEARCH_SECS);
                    return Ok(res);
                }
                Err(e) => self.primary_breaker.record_failure(&e),
            }
        }
        if let Some(secondary) = &self.secondary_browse
            && let Ok(res) = secondary.search(query).await
        {
            self.write_swr(&cache_key, &res, TTL_HYBRID_SEARCH_SECS);
            return Ok(res);
        }
        if let Some(inner) = &self.inner {
            match inner.search(query).await {
                Ok(res) => {
                    self.write_swr(&cache_key, &res, TTL_HYBRID_SEARCH_SECS);
                    return Ok(res);
                }
                Err(e) => return cached.ok_or(e),
            }
        }
        cached.ok_or_else(|| {
            CoreError::Provider(ProviderError::Parse("no search provider configured".into()))
        })
    }

    async fn home(&self) -> CoreResult<Vec<HomeSection>> {
        let cache_key = "hybrid:home:v1";
        let (cached, is_stale) = self.read_swr::<Vec<HomeSection>>(cache_key);
        if let Some(fresh) = cached.as_ref().filter(|s| !is_stale && !s.is_empty()) {
            return Ok(fresh.clone());
        }

        if !self.primary_breaker.is_open()
            && let Some(primary) = &self.primary_browse
        {
            match primary.home().await {
                Ok(sections) if !sections.is_empty() => {
                    self.primary_breaker.record_success();
                    self.write_swr(cache_key, &sections, TTL_HYBRID_HOME_SECS);
                    return Ok(sections);
                }
                Ok(_) => {}
                Err(e) => self.primary_breaker.record_failure(&e),
            }
        }
        if let Some(secondary) = &self.secondary_browse
            && let Ok(sections) = secondary.home().await
            && !sections.is_empty()
        {
            self.write_swr(cache_key, &sections, TTL_HYBRID_HOME_SECS);
            return Ok(sections);
        }
        if let Some(inner) = &self.inner {
            match inner.home().await {
                Ok(sections) => {
                    if !sections.is_empty() {
                        self.write_swr(cache_key, &sections, TTL_HYBRID_HOME_SECS);
                    }
                    return Ok(sections);
                }
                Err(e) => return cached.ok_or(e),
            }
        }
        cached.ok_or_else(|| {
            CoreError::Provider(ProviderError::Parse("no home provider configured".into()))
        })
    }

    async fn browse_collection(&self, browse_id: &str) -> CoreResult<Playlist> {
        let cache_key = format!("hybrid:collection:{browse_id}");
        let (cached, is_stale) = self.read_swr::<Playlist>(&cache_key);
        if let Some(fresh) = cached.as_ref().filter(|_| !is_stale) {
            return Ok(fresh.clone());
        }

        if let Some(primary) = &self.primary_browse
            && let Ok(pl) = primary.browse_collection(browse_id).await
        {
            self.write_swr(&cache_key, &pl, TTL_HYBRID_COLLECTION_SECS);
            return Ok(pl);
        }
        if let Some(secondary) = &self.secondary_browse
            && let Ok(pl) = secondary.browse_collection(browse_id).await
        {
            self.write_swr(&cache_key, &pl, TTL_HYBRID_COLLECTION_SECS);
            return Ok(pl);
        }
        if let Some(inner) = &self.inner {
            match inner.browse_collection(browse_id).await {
                Ok(pl) => {
                    self.write_swr(&cache_key, &pl, TTL_HYBRID_COLLECTION_SECS);
                    return Ok(pl);
                }
                Err(e) => return cached.ok_or(e),
            }
        }
        cached.ok_or_else(|| {
            CoreError::Provider(ProviderError::Parse(
                "no collection provider configured".into(),
            ))
        })
    }

    async fn liked_songs(&self) -> CoreResult<Vec<Track>> {
        if let Some(primary) = &self.primary_browse
            && let Ok(tracks) = primary.liked_songs().await
        {
            return Ok(tracks);
        }
        if let Some(inner) = &self.inner {
            return inner.liked_songs().await;
        }
        Ok(Vec::new())
    }

    async fn autoplay_next(&self, video_id: &str) -> CoreResult<Vec<Track>> {
        if let Some(primary) = &self.primary_browse
            && let Ok(tracks) = primary.autoplay_next(video_id).await
        {
            return Ok(tracks);
        }
        if let Some(secondary) = &self.secondary_browse
            && let Ok(tracks) = secondary.autoplay_next(video_id).await
        {
            return Ok(tracks);
        }
        if let Some(inner) = &self.inner {
            return inner.autoplay_next(video_id).await;
        }
        Ok(Vec::new())
    }

    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo> {
        let mut last_err = None;
        if let Some(primary) = &self.primary_playback {
            match primary.resolve_stream(video_id).await {
                Ok(stream) => return Ok(stream),
                Err(e) => last_err = Some(e),
            }
        }
        if let Some(secondary) = &self.secondary_playback {
            match secondary.resolve_stream(video_id).await {
                Ok(stream) => return Ok(stream),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            CoreError::Provider(ProviderError::NoStream {
                video_id: video_id.to_owned(),
            })
        }))
    }

    async fn ytm_lyrics(&self, video_id: &str) -> CoreResult<Option<LyricsDocument>> {
        if let Some(primary) = &self.primary_browse
            && let Ok(Some(doc)) = primary.ytm_lyrics(video_id).await
        {
            return Ok(Some(doc));
        }
        if let Some(secondary) = &self.secondary_browse
            && let Ok(Some(doc)) = secondary.ytm_lyrics(video_id).await
        {
            return Ok(Some(doc));
        }
        if let Some(inner) = &self.inner {
            return inner.ytm_lyrics(video_id).await;
        }
        Ok(None)
    }
}

#[async_trait]
impl CatalogExtProvider for HybridMusicProvider {
    async fn explore(&self) -> CoreResult<ExplorePage> {
        #[cfg(feature = "provider-ytmapi")]
        let ytmapi_page = if let Some(ytmapi) = &self.ytmapi {
            ytmapi.explore().await.ok()
        } else {
            None
        };
        #[cfg(not(feature = "provider-ytmapi"))]
        let ytmapi_page: Option<ExplorePage> = None;

        #[cfg(feature = "provider-rustypipe")]
        let rp_page = if let Some(rp) = &self.rp_browse {
            rp.explore_fallback().await.ok()
        } else {
            None
        };
        #[cfg(not(feature = "provider-rustypipe"))]
        let rp_page: Option<ExplorePage> = None;

        let mut page = ExplorePage {
            new_releases: rp_page
                .as_ref()
                .map(|p| p.new_releases.clone())
                .unwrap_or_default(),
            charts: rp_page.map(|p| p.charts).unwrap_or_default(),
            moods_and_genres: ytmapi_page.map(|p| p.moods_and_genres).unwrap_or_default(),
        };

        if page.new_releases.is_empty()
            && page.charts.is_empty()
            && page.moods_and_genres.is_empty()
            && let Some(inner) = &self.inner
        {
            let sections = inner.explore().await.unwrap_or_default();
            page.charts = sections;
        }
        Ok(page)
    }

    async fn mood_playlists(&self, params: &str) -> CoreResult<Vec<LibraryCollectionRef>> {
        #[cfg(feature = "provider-ytmapi")]
        if let Some(ytmapi) = &self.ytmapi
            && let Ok(list) = ytmapi.mood_playlists(params).await
        {
            return Ok(list);
        }
        let sr = self.search(params).await?;
        Ok(sr
            .albums
            .into_iter()
            .map(|p| LibraryCollectionRef {
                browse_id: p.id.clone(),
                playlist_id: Some(p.id),
                title: p.title,
                subtitle: p.subtitle.unwrap_or_default(),
                artwork_url: p.artwork_url,
                track_count: None,
            })
            .collect())
    }

    async fn search_categorized(
        &self,
        query: &str,
        category: SearchCategory,
        continuation: Option<&str>,
    ) -> CoreResult<SearchResults> {
        #[cfg(feature = "provider-ytmapi")]
        if let Some(ytmapi) = &self.ytmapi
            && let Ok(res) = ytmapi
                .search_categorized(query, category, continuation)
                .await
        {
            return Ok(res);
        }
        #[cfg(feature = "provider-rustypipe")]
        if let Some(rp) = &self.rp_browse
            && let Ok(res) = rp.search_categorized(query, category).await
        {
            return Ok(res);
        }
        let mut res = self.search(query).await?;
        res.tracks = deduplicate_tracks(res.tracks);
        res.albums = deduplicate_playlists(res.albums);
        Ok(res)
    }

    async fn search_suggestions(&self, prefix: &str) -> CoreResult<Vec<String>> {
        #[cfg(feature = "provider-ytmapi")]
        if let Some(ytmapi) = &self.ytmapi
            && let Ok(list) = ytmapi.search_suggestions(prefix).await
        {
            return Ok(list);
        }
        #[cfg(feature = "provider-rustypipe")]
        if let Some(rp) = &self.rp_browse
            && let Ok(list) = rp.search_suggestions(prefix).await
        {
            return Ok(list);
        }
        Ok(Vec::new())
    }

    async fn fetch_library_snapshot(&self) -> CoreResult<LibrarySnapshot> {
        #[cfg(feature = "provider-ytmapi")]
        if let Some(ytmapi) = &self.ytmapi {
            return ytmapi.fetch_library_snapshot().await;
        }
        let liked = self.liked_songs().await.unwrap_or_default();
        Ok(LibrarySnapshot {
            liked_songs: liked,
            fetched_at_unix_secs: unix_secs(),
            ..Default::default()
        })
    }

    async fn fetch_artist(&self, channel_id: &str) -> CoreResult<ArtistDetail> {
        #[cfg(feature = "provider-ytmapi")]
        if let Some(ytmapi) = &self.ytmapi
            && let Ok(detail) = ytmapi.fetch_artist(channel_id).await
        {
            return Ok(detail);
        }
        #[cfg(feature = "provider-rustypipe")]
        if let Some(rp) = &self.rp_browse {
            return rp.fetch_artist(channel_id).await;
        }
        Err(CoreError::Provider(ProviderError::Parse(
            "no provider available for fetch_artist".into(),
        )))
    }

    async fn fetch_album(&self, browse_id: &str) -> CoreResult<AlbumDetail> {
        #[cfg(feature = "provider-ytmapi")]
        if let Some(ytmapi) = &self.ytmapi
            && let Ok(detail) = ytmapi.fetch_album(browse_id).await
        {
            return Ok(detail);
        }
        #[cfg(feature = "provider-rustypipe")]
        if let Some(rp) = &self.rp_browse {
            return rp.fetch_album(browse_id).await;
        }
        let pl = self.browse_collection(browse_id).await?;
        let total_duration_ms = pl.tracks.iter().map(|t| t.duration_ms).sum();
        Ok(AlbumDetail {
            browse_id: pl.id,
            audio_playlist_id: None,
            title: pl.title,
            artists: Vec::new(),
            release_year: None,
            release_type: "Album".to_owned(),
            description: pl.subtitle,
            artwork_url: pl.artwork_url,
            total_duration_ms,
            tracks: pl.tracks,
            other_versions: Vec::new(),
        })
    }

    async fn create_playlist(
        &self,
        title: &str,
        description: &str,
        video_ids: &[String],
    ) -> CoreResult<String> {
        #[cfg(feature = "provider-ytmapi")]
        if let Some(ytmapi) = &self.ytmapi {
            return ytmapi.create_playlist(title, description, video_ids).await;
        }
        Err(CoreError::Provider(ProviderError::Parse(
            "create_playlist requires provider-ytmapi".into(),
        )))
    }
}

#[async_trait]
impl InnerTubeMutations for HybridMusicProvider {
    async fn report_atr(&self, report: &WatchtimeReport) -> Result<MutationReceipt, MutationError> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| MutationError::InvalidInput("no InnerTubeClient configured".into()))?;
        inner.report_atr(report).await
    }

    async fn set_like(
        &self,
        video_id: &str,
        action: LikeAction,
    ) -> Result<MutationReceipt, MutationError> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| MutationError::InvalidInput("no InnerTubeClient configured".into()))?;
        inner.set_like(video_id, action).await
    }

    async fn edit_playlist(
        &self,
        playlist_id: &str,
        op: &PlaylistEditOp,
    ) -> Result<MutationReceipt, MutationError> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| MutationError::InvalidInput("no InnerTubeClient configured".into()))?;
        inner.edit_playlist(playlist_id, op).await
    }
}
