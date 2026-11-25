//! Anonymous-only `rustypipe` (GPL-3.0) fallback browse & playback adapter.
//!
//! Security invariant: `RustyPipeBrowseAdapter` and `RustyPipePlaybackAdapter`
//! NEVER receive browser cookies (`SAPISID` / `Cookie` headers). They operate
//! strictly in anonymous mode for resilient browse and `/player` stream fallback.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use resonance_contract::{
    AlbumDetail, ArtistDetail, AudioCodec, AuthError, CoreError, CoreResult, ExplorePage, HomeItem,
    HomeSection, LyricLine, LyricSyncKind, LyricsDocument, Playlist, ProviderError, SearchCategory,
    SearchResults, StreamInfo, Track,
};
use rustypipe::client::{ClientType, RustyPipe};
use rustypipe::model::{AudioCodec as RpAudioCodec, AudioStream};

use crate::innertube::formats::TargetPlatform;
use crate::innertube::matrix::AccountTier;
use crate::innertube::provider_adapter::{BrowseDelegate, PlaybackResolver};
use crate::providers::mappers::rustypipe_map as map;
use crate::providers::mappers::{deduplicate_playlists, deduplicate_tracks};
use crate::unix_secs;

/// Anonymous `rustypipe` browse & metadata fallback adapter.
pub struct RustyPipeBrowseAdapter {
    rp: Arc<RustyPipe>,
}

impl RustyPipeBrowseAdapter {
    pub fn new(storage_dir: Option<PathBuf>) -> Self {
        let mut builder = RustyPipe::builder();
        if let Some(dir) = storage_dir {
            builder = builder.storage_dir(dir);
        }
        let rp = builder.build().unwrap_or_else(|_| RustyPipe::new());
        Self { rp: Arc::new(rp) }
    }

    pub fn shared_client(&self) -> Arc<RustyPipe> {
        Arc::clone(&self.rp)
    }

    pub async fn explore_fallback(&self) -> CoreResult<ExplorePage> {
        let q = self.rp.query();
        let (charts_res, new_albums_res) = tokio::join!(q.music_charts(None), q.music_new_albums());
        let charts = charts_res.map(map::map_charts).unwrap_or_default();
        let new_releases: Vec<Playlist> = new_albums_res
            .map(|albums| albums.into_iter().map(map::map_album_item).collect())
            .unwrap_or_default();

        if charts.is_empty() && new_releases.is_empty() {
            return Err(CoreError::Provider(ProviderError::Network(
                "rustypipe explore fallback returned empty charts and new releases".into(),
            )));
        }

        Ok(ExplorePage {
            new_releases,
            charts,
            moods_and_genres: Vec::new(),
        })
    }

    pub async fn search_categorized(
        &self,
        query: &str,
        category: SearchCategory,
    ) -> CoreResult<SearchResults> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(SearchResults {
                query: String::new(),
                tracks: Vec::new(),
                albums: Vec::new(),
                continuation: None,
            });
        }

        let q = self.rp.query();
        match category {
            SearchCategory::All => {
                let (tracks_res, albums_res) = tokio::join!(
                    q.music_search_tracks(trimmed),
                    q.music_search_albums(trimmed),
                );
                let tracks = tracks_res
                    .map(|r| {
                        deduplicate_tracks(r.items.items.into_iter().map(map::map_track).collect())
                    })
                    .unwrap_or_default();
                let albums = albums_res
                    .map(|r| {
                        deduplicate_playlists(
                            r.items.items.into_iter().map(map::map_album_item).collect(),
                        )
                    })
                    .unwrap_or_default();
                Ok(SearchResults {
                    query: trimmed.to_owned(),
                    tracks,
                    albums,
                    continuation: None,
                })
            }
            SearchCategory::Songs => {
                let res = q
                    .music_search_tracks(trimmed)
                    .await
                    .map_err(|e| CoreError::Provider(ProviderError::Network(e.to_string())))?;
                Ok(SearchResults {
                    query: trimmed.to_owned(),
                    tracks: deduplicate_tracks(
                        res.items.items.into_iter().map(map::map_track).collect(),
                    ),
                    albums: Vec::new(),
                    continuation: res.items.ctoken,
                })
            }
            SearchCategory::Albums => {
                let res = q
                    .music_search_albums(trimmed)
                    .await
                    .map_err(|e| CoreError::Provider(ProviderError::Network(e.to_string())))?;
                Ok(SearchResults {
                    query: trimmed.to_owned(),
                    tracks: Vec::new(),
                    albums: deduplicate_playlists(
                        res.items
                            .items
                            .into_iter()
                            .map(map::map_album_item)
                            .collect(),
                    ),
                    continuation: res.items.ctoken,
                })
            }
            SearchCategory::Artists => {
                let res = q
                    .music_search_artists(trimmed)
                    .await
                    .map_err(|e| CoreError::Provider(ProviderError::Network(e.to_string())))?;
                Ok(SearchResults {
                    query: trimmed.to_owned(),
                    tracks: Vec::new(),
                    albums: deduplicate_playlists(
                        res.items
                            .items
                            .into_iter()
                            .map(map::map_artist_item)
                            .collect(),
                    ),
                    continuation: res.items.ctoken,
                })
            }
            SearchCategory::Playlists | SearchCategory::Podcasts => {
                let res = q
                    .music_search_playlists(trimmed, false)
                    .await
                    .map_err(|e| CoreError::Provider(ProviderError::Network(e.to_string())))?;
                Ok(SearchResults {
                    query: trimmed.to_owned(),
                    tracks: Vec::new(),
                    albums: deduplicate_playlists(
                        res.items
                            .items
                            .into_iter()
                            .map(map::map_playlist_item)
                            .collect(),
                    ),
                    continuation: res.items.ctoken,
                })
            }
        }
    }

    pub async fn search_suggestions(&self, prefix: &str) -> CoreResult<Vec<String>> {
        let trimmed = prefix.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        self.rp
            .query()
            .music_search_suggestion(trimmed)
            .await
            .map(|s| s.terms)
            .map_err(|e| CoreError::Provider(ProviderError::Network(e.to_string())))
    }

    pub async fn fetch_artist(&self, channel_id: &str) -> CoreResult<ArtistDetail> {
        let artist = self
            .rp
            .query()
            .music_artist(channel_id, false)
            .await
            .map_err(|e| CoreError::Provider(ProviderError::Network(e.to_string())))?;
        Ok(map::map_music_artist(artist))
    }

    pub async fn fetch_album(&self, browse_id: &str) -> CoreResult<AlbumDetail> {
        let album = self
            .rp
            .query()
            .music_album(browse_id)
            .await
            .map_err(|e| CoreError::Provider(ProviderError::Network(e.to_string())))?;
        Ok(map::map_music_album(album))
    }
}

#[async_trait]
impl BrowseDelegate for RustyPipeBrowseAdapter {
    async fn search(&self, query: &str) -> CoreResult<SearchResults> {
        self.search_categorized(query, SearchCategory::All).await
    }

    async fn home(&self) -> CoreResult<Vec<HomeSection>> {
        let q = self.rp.query();
        let (charts_res, new_albums_res) = tokio::join!(q.music_charts(None), q.music_new_albums());

        let mut sections = charts_res.map(map::map_charts).unwrap_or_default();
        if let Ok(albums) = new_albums_res {
            if !albums.is_empty() {
                sections.insert(
                    0,
                    HomeSection {
                        id: "rp-new-releases".to_owned(),
                        title: "New releases".to_owned(),
                        items: albums
                            .into_iter()
                            .map(|a| {
                                let pl = map::map_album_item(a);
                                HomeItem::Collection {
                                    browse_id: pl.id,
                                    title: pl.title,
                                    subtitle: pl.subtitle.unwrap_or_default(),
                                    artwork_url: pl.artwork_url,
                                }
                            })
                            .collect(),
                    },
                );
            }
        }

        if sections.is_empty() {
            return Err(CoreError::Provider(ProviderError::Network(
                "rustypipe home fallback returned no sections".into(),
            )));
        }
        Ok(sections)
    }

    async fn browse_collection(&self, browse_id: &str) -> CoreResult<Playlist> {
        if browse_id.starts_with("MPREb_") || browse_id.starts_with("OLAK5uy_") {
            let album = self.fetch_album(browse_id).await?;
            let artists = album
                .artists
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Ok(Playlist {
                id: album.browse_id,
                title: album.title,
                subtitle: (!artists.is_empty()).then_some(artists),
                artwork_url: album.artwork_url,
                tracks: album.tracks,
            })
        } else {
            let pl = self
                .rp
                .query()
                .music_playlist(browse_id)
                .await
                .map_err(|e| CoreError::Provider(ProviderError::Network(e.to_string())))?;
            Ok(map::map_music_playlist(pl))
        }
    }

    async fn liked_songs(&self) -> CoreResult<Vec<Track>> {
        Err(CoreError::Auth(AuthError::NotSignedIn))
    }

    async fn autoplay_next(&self, video_id: &str) -> CoreResult<Vec<Track>> {
        let radio = self
            .rp
            .query()
            .music_radio_track(video_id)
            .await
            .map_err(|e| CoreError::Provider(ProviderError::Network(e.to_string())))?;
        Ok(deduplicate_tracks(
            radio.items.into_iter().map(map::map_track).collect(),
        ))
    }

    async fn ytm_lyrics(&self, video_id: &str) -> CoreResult<Option<LyricsDocument>> {
        let details = match self.rp.query().music_details(video_id).await {
            Ok(d) => d,
            Err(_) => return Ok(None),
        };
        let Some(lyrics_id) = details.lyrics_id else {
            return Ok(None);
        };
        let lyrics = match self.rp.query().music_lyrics(&lyrics_id).await {
            Ok(l) => l,
            Err(_) => return Ok(None),
        };
        let lines = lyrics
            .body
            .lines()
            .map(|line| LyricLine {
                start_ms: 0,
                end_ms: 0,
                text: line.to_owned(),
                words: Vec::new(),
                is_instrumental: line.trim().is_empty(),
            })
            .collect();
        Ok(Some(LyricsDocument {
            video_id: video_id.to_owned(),
            provider: "YouTube Music (rustypipe)".to_owned(),
            sync_kind: LyricSyncKind::Plain,
            lines,
        }))
    }
}

/// Selects the best `rustypipe` `AudioStream` according to Resonance's
/// `TargetPlatform` decoder matrix and `AccountTier` (iOS strictly excludes Opus).
pub fn select_rustypipe_stream(
    video_id: &str,
    streams: &[AudioStream],
    platform: TargetPlatform,
    tier: AccountTier,
    fallback_duration_ms: u64,
) -> CoreResult<StreamInfo> {
    let compatible: Vec<&AudioStream> = streams
        .iter()
        .filter(|s| {
            !s.url.is_empty()
                && s.drm_track_type.is_none()
                && (platform.supports_webm_opus()
                    || (!matches!(s.codec, RpAudioCodec::Opus) && s.itag != 774 && s.itag != 251))
        })
        .collect();

    if compatible.is_empty() {
        return Err(CoreError::Provider(ProviderError::NoStream {
            video_id: video_id.to_owned(),
        }));
    }

    let preferred = platform.preferred_audio_itags(tier);
    let chosen = preferred
        .iter()
        .find_map(|&itag| compatible.iter().copied().find(|s| s.itag == itag))
        .or_else(|| compatible.iter().copied().max_by_key(|s| s.bitrate))
        .ok_or_else(|| {
            CoreError::Provider(ProviderError::NoStream {
                video_id: video_id.to_owned(),
            })
        })?;

    let codec = match chosen.codec {
        RpAudioCodec::Opus => AudioCodec::Opus,
        _ => AudioCodec::Aac,
    };

    Ok(StreamInfo {
        video_id: video_id.to_owned(),
        url: chosen.url.clone(),
        itag: chosen.itag,
        mime_type: chosen.mime.clone(),
        codec,
        bitrate_kbps: chosen.bitrate / 1000,
        content_length: chosen.size,
        duration_ms: chosen
            .duration_ms
            .map(u64::from)
            .unwrap_or(fallback_duration_ms),
        loudness_db: chosen.loudness_db,
        resolved_by_client: "RUSTYPIPE_ANON".to_owned(),
        expires_at_unix_secs: unix_secs().saturating_add(5 * 3600),
    })
}

/// Anonymous `rustypipe` playback fallback resolver (`PlaybackResolver`).
pub struct RustyPipePlaybackAdapter {
    rp: Arc<RustyPipe>,
    platform: TargetPlatform,
    tier: AccountTier,
}

impl RustyPipePlaybackAdapter {
    pub fn new(rp: Arc<RustyPipe>) -> Self {
        Self {
            rp,
            platform: TargetPlatform::current(),
            tier: AccountTier::Anonymous,
        }
    }

    pub fn with_platform_and_tier(
        rp: Arc<RustyPipe>,
        platform: TargetPlatform,
        tier: AccountTier,
    ) -> Self {
        Self { rp, platform, tier }
    }
}

#[async_trait]
impl PlaybackResolver for RustyPipePlaybackAdapter {
    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo> {
        let client_type = match self.platform {
            TargetPlatform::Android => ClientType::Android,
            TargetPlatform::Desktop | TargetPlatform::Ios => ClientType::Desktop,
        };
        let player = self
            .rp
            .query()
            .player_from_client(video_id, client_type)
            .await
            .map_err(|e| CoreError::Provider(ProviderError::Network(e.to_string())))?;

        let duration_ms = u64::from(player.details.duration).saturating_mul(1000);
        select_rustypipe_stream(
            video_id,
            &player.audio_streams,
            self.platform,
            self.tier,
            duration_ms,
        )
    }
}
