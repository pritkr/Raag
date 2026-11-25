//! `HybridMusicProvider` composing:
//! 1. Browse/Catalog failover: `YtmapiBrowseAdapter` -> `RustyPipeBrowseAdapter` -> `InnerTubeClient` (+ SWR cache)
//! 2. Playback failover: `InnerTubeClient` (`ClientMatrix` + `rquickjs` + PO-Token) -> `RustyPipePlaybackAdapter`
//! 3. Mutations: `InnerTubeClient` (`InnerTubeMutations`)

use std::sync::Arc;

use async_trait::async_trait;
use resonance_contract::{
    AlbumDetail, ArtistDetail, AuthSession, CacheStore, CatalogExtProvider, CoreError, CoreResult,
    ExplorePage, HomeSection, InnerTubeMutations, LibraryCollectionRef, LibrarySnapshot,
    LikeAction, LyricsDocument, MusicProvider, MutationError, MutationReceipt, Playlist,
    PlaylistEditOp, ProviderError, SearchCategory, SearchResults, StreamInfo, Track,
    WatchtimeReport,
};
use serde::{Deserialize, Serialize};

use crate::innertube::client::InnerTubeClient;
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

/// Hybrid provider orchestrating `ytmapi-rs`, `rustypipe`, and `InnerTubeClient`.
pub struct HybridMusicProvider {
    inner: Arc<InnerTubeClient>,
    cache: Arc<dyn CacheStore>,
    #[cfg(feature = "provider-ytmapi")]
    ytmapi: Option<Arc<YtmapiBrowseAdapter>>,
    #[cfg(feature = "provider-rustypipe")]
    rp_browse: Option<Arc<RustyPipeBrowseAdapter>>,
    #[cfg(feature = "provider-rustypipe")]
    rp_playback: Option<Arc<RustyPipePlaybackAdapter>>,
    primary_browse: Option<Arc<dyn BrowseDelegate>>,
    secondary_browse: Option<Arc<dyn BrowseDelegate>>,
    primary_playback: Option<Arc<dyn PlaybackResolver>>,
    secondary_playback: Option<Arc<dyn PlaybackResolver>>,
}

impl HybridMusicProvider {
    pub fn new(inner: Arc<InnerTubeClient>, cache: Arc<dyn CacheStore>) -> Self {
        #[cfg(feature = "provider-ytmapi")]
        let ytmapi = Some(Arc::new(YtmapiBrowseAdapter::new(Arc::clone(&cache))));

        #[cfg(feature = "provider-rustypipe")]
        let (rp_browse, rp_playback) = {
            let browse = Arc::new(RustyPipeBrowseAdapter::new(None));
            let playback = Arc::new(RustyPipePlaybackAdapter::new(browse.shared_client()));
            (Some(browse), Some(playback))
        };

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

        let secondary_playback: Option<Arc<dyn PlaybackResolver>> = {
            #[cfg(feature = "provider-rustypipe")]
            {
                rp_playback
                    .as_ref()
                    .map(|a| Arc::clone(a) as Arc<dyn PlaybackResolver>)
            }
            #[cfg(not(feature = "provider-rustypipe"))]
            {
                None
            }
        };

        Self {
            primary_playback: Some(Arc::clone(&inner) as Arc<dyn PlaybackResolver>),
            inner,
            cache,
            #[cfg(feature = "provider-ytmapi")]
            ytmapi,
            #[cfg(feature = "provider-rustypipe")]
            rp_browse,
            #[cfg(feature = "provider-rustypipe")]
            rp_playback,
            primary_browse,
            secondary_browse,
            secondary_playback,
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
        Self {
            inner,
            cache,
            #[cfg(feature = "provider-ytmapi")]
            ytmapi: None,
            #[cfg(feature = "provider-rustypipe")]
            rp_browse: None,
            #[cfg(feature = "provider-rustypipe")]
            rp_playback: None,
            primary_browse,
            secondary_browse,
            primary_playback,
            secondary_playback,
        }
    }

    /// Synchronizes `AuthSession` into `InnerTubeClient` and `YtmapiBrowseAdapter`.
    /// Note: `rustypipe` adapters intentionally NEVER receive browser cookies.
    pub async fn sync_session(&self, session: Option<&AuthSession>) -> CoreResult<()> {
        if let Some(s) = session {
            let _ = self.inner.sign_in(s.clone());
        } else {
            let _ = self.inner.sign_out();
        }
        #[cfg(feature = "provider-ytmapi")]
        if let Some(ytmapi) = &self.ytmapi {
            ytmapi.sync_session(session).await?;
        }
        Ok(())
    }

    fn read_swr<T: for<'de> Deserialize<'de>>(&self, key: &str) -> (Option<T>, bool) {
        let now = unix_secs();
        match self.cache.get_metadata(key, now) {
            Ok(Some((json, is_fresh))) => match serde_json::from_str::<T>(&json) {
                Ok(val) => (Some(val), is_fresh),
                Err(_) => {
                    let _ = self.cache.evict_metadata(key);
                    (None, false)
                }
            },
            _ => (None, false),
        }
    }

    fn write_swr<T: Serialize>(&self, key: &str, val: &T, ttl_secs: u64) {
        if let Ok(json) = serde_json::to_string(val) {
            let _ = self.cache.put_metadata(key, &json, ttl_secs, unix_secs());
        }
    }
}

#[async_trait]
impl MusicProvider for HybridMusicProvider {
    async fn search(&self, query: &str) -> CoreResult<SearchResults> {
        let cache_key = format!("hybrid:search:{}", query.trim());
        let (stale, _) = self.read_swr::<SearchResults>(&cache_key);

        if let Some(primary) = &self.primary_browse {
            if let Ok(res) = primary.search(query).await {
                self.write_swr(&cache_key, &res, TTL_HYBRID_SEARCH_SECS);
                return Ok(res);
            }
        }
        if let Some(secondary) = &self.secondary_browse {
            if let Ok(res) = secondary.search(query).await {
                self.write_swr(&cache_key, &res, TTL_HYBRID_SEARCH_SECS);
                return Ok(res);
            }
        }
        match self.inner.search(query).await {
            Ok(res) => {
                self.write_swr(&cache_key, &res, TTL_HYBRID_SEARCH_SECS);
                Ok(res)
            }
            Err(e) => stale.ok_or(e),
        }
    }

    async fn home(&self) -> CoreResult<Vec<HomeSection>> {
        let cache_key = "hybrid:home:v1";
        let (stale, _) = self.read_swr::<Vec<HomeSection>>(cache_key);

        if let Some(primary) = &self.primary_browse {
            if let Ok(sections) = primary.home().await {
                if !sections.is_empty() {
                    self.write_swr(cache_key, &sections, TTL_HYBRID_HOME_SECS);
                    return Ok(sections);
                }
            }
        }
        if let Some(secondary) = &self.secondary_browse {
            if let Ok(sections) = secondary.home().await {
                if !sections.is_empty() {
                    self.write_swr(cache_key, &sections, TTL_HYBRID_HOME_SECS);
                    return Ok(sections);
                }
            }
        }
        match self.inner.home().await {
            Ok(sections) => {
                self.write_swr(cache_key, &sections, TTL_HYBRID_HOME_SECS);
                Ok(sections)
            }
            Err(e) => stale.ok_or(e),
        }
    }

    async fn browse_collection(&self, browse_id: &str) -> CoreResult<Playlist> {
        let cache_key = format!("hybrid:collection:{browse_id}");
        let (stale, _) = self.read_swr::<Playlist>(&cache_key);

        if let Some(primary) = &self.primary_browse {
            if let Ok(pl) = primary.browse_collection(browse_id).await {
                self.write_swr(&cache_key, &pl, TTL_HYBRID_COLLECTION_SECS);
                return Ok(pl);
            }
        }
        if let Some(secondary) = &self.secondary_browse {
            if let Ok(pl) = secondary.browse_collection(browse_id).await {
                self.write_swr(&cache_key, &pl, TTL_HYBRID_COLLECTION_SECS);
                return Ok(pl);
            }
        }
        match self.inner.browse_collection(browse_id).await {
            Ok(pl) => {
                self.write_swr(&cache_key, &pl, TTL_HYBRID_COLLECTION_SECS);
                Ok(pl)
            }
            Err(e) => stale.ok_or(e),
        }
    }

    async fn liked_songs(&self) -> CoreResult<Vec<Track>> {
        if let Some(primary) = &self.primary_browse {
            if let Ok(tracks) = primary.liked_songs().await {
                return Ok(tracks);
            }
        }
        self.inner.liked_songs().await
    }

    async fn autoplay_next(&self, video_id: &str) -> CoreResult<Vec<Track>> {
        if let Some(primary) = &self.primary_browse {
            if let Ok(tracks) = primary.autoplay_next(video_id).await {
                return Ok(tracks);
            }
        }
        if let Some(secondary) = &self.secondary_browse {
            if let Ok(tracks) = secondary.autoplay_next(video_id).await {
                return Ok(tracks);
            }
        }
        self.inner.autoplay_next(video_id).await
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
        if let Some(primary) = &self.primary_browse {
            if let Ok(Some(doc)) = primary.ytm_lyrics(video_id).await {
                return Ok(Some(doc));
            }
        }
        if let Some(secondary) = &self.secondary_browse {
            if let Ok(Some(doc)) = secondary.ytm_lyrics(video_id).await {
                return Ok(Some(doc));
            }
        }
        self.inner.ytm_lyrics(video_id).await
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
        {
            let sections = self.inner.explore().await.unwrap_or_default();
            page.charts = sections;
        }
        Ok(page)
    }

    async fn mood_playlists(&self, params: &str) -> CoreResult<Vec<LibraryCollectionRef>> {
        #[cfg(feature = "provider-ytmapi")]
        if let Some(ytmapi) = &self.ytmapi {
            if let Ok(list) = ytmapi.mood_playlists(params).await {
                return Ok(list);
            }
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
        if let Some(ytmapi) = &self.ytmapi {
            if let Ok(res) = ytmapi
                .search_categorized(query, category, continuation)
                .await
            {
                return Ok(res);
            }
        }
        #[cfg(feature = "provider-rustypipe")]
        if let Some(rp) = &self.rp_browse {
            if let Ok(res) = rp.search_categorized(query, category).await {
                return Ok(res);
            }
        }
        let mut res = self.inner.search(query).await?;
        res.tracks = deduplicate_tracks(res.tracks);
        res.albums = deduplicate_playlists(res.albums);
        Ok(res)
    }

    async fn search_suggestions(&self, prefix: &str) -> CoreResult<Vec<String>> {
        #[cfg(feature = "provider-ytmapi")]
        if let Some(ytmapi) = &self.ytmapi {
            if let Ok(list) = ytmapi.search_suggestions(prefix).await {
                return Ok(list);
            }
        }
        #[cfg(feature = "provider-rustypipe")]
        if let Some(rp) = &self.rp_browse {
            if let Ok(list) = rp.search_suggestions(prefix).await {
                return Ok(list);
            }
        }
        Ok(Vec::new())
    }

    async fn fetch_library_snapshot(&self) -> CoreResult<LibrarySnapshot> {
        #[cfg(feature = "provider-ytmapi")]
        if let Some(ytmapi) = &self.ytmapi {
            return ytmapi.fetch_library_snapshot().await;
        }
        let liked = self.inner.liked_songs().await.unwrap_or_default();
        Ok(LibrarySnapshot {
            liked_songs: liked,
            fetched_at_unix_secs: unix_secs(),
            ..Default::default()
        })
    }

    async fn fetch_artist(&self, channel_id: &str) -> CoreResult<ArtistDetail> {
        #[cfg(feature = "provider-ytmapi")]
        if let Some(ytmapi) = &self.ytmapi {
            if let Ok(detail) = ytmapi.fetch_artist(channel_id).await {
                return Ok(detail);
            }
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
        if let Some(ytmapi) = &self.ytmapi {
            if let Ok(detail) = ytmapi.fetch_album(browse_id).await {
                return Ok(detail);
            }
        }
        #[cfg(feature = "provider-rustypipe")]
        if let Some(rp) = &self.rp_browse {
            return rp.fetch_album(browse_id).await;
        }
        let pl = self.inner.browse_collection(browse_id).await?;
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
        self.inner.report_atr(report).await
    }

    async fn set_like(
        &self,
        video_id: &str,
        action: LikeAction,
    ) -> Result<MutationReceipt, MutationError> {
        self.inner.set_like(video_id, action).await
    }

    async fn edit_playlist(
        &self,
        playlist_id: &str,
        op: &PlaylistEditOp,
    ) -> Result<MutationReceipt, MutationError> {
        self.inner.edit_playlist(playlist_id, op).await
    }
}
