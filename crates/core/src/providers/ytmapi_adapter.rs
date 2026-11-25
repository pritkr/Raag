//! `ytmapi-rs` (MIT) browse, search, library, artist/album, and playlist adapter
//! with SWR SQLite caching and auth session synchronization.

use std::sync::Arc;

use async_trait::async_trait;
use resonance_contract::{
    AlbumDetail, ArtistDetail, AuthError, AuthSession, CacheStore, CatalogExtProvider, CoreError,
    CoreResult, ExplorePage, HomeSection, LibraryCollectionRef, LibrarySnapshot, LyricsDocument,
    Playlist, ProviderError, SearchCategory, SearchResults, Track,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use ytmapi_rs::YtMusic;
use ytmapi_rs::auth::{BrowserToken, OAuthToken};
use ytmapi_rs::common::{
    AlbumID, ArtistChannelID, LyricsID, MoodCategoryParams, PlaylistID, TextRun, VideoID, YoutubeID,
};
use ytmapi_rs::parse::{Lyrics, SearchResultAlbum, TableListUploadSong};
use ytmapi_rs::query::playlist::additems::DuplicateHandlingMode;
use ytmapi_rs::query::playlist::{AddPlaylistItemsQuery, PrivacyStatus};
use ytmapi_rs::query::search::{
    AlbumsFilter, ArtistsFilter, CommunityPlaylistsFilter, PodcastsFilter, SongsFilter,
};
use ytmapi_rs::query::{
    CreatePlaylistQuery, GetAlbumQuery, GetArtistQuery, GetHistoryQuery, GetLibraryAlbumsQuery,
    GetLibraryArtistsQuery, GetLibraryPlaylistsQuery, GetLibraryPodcastsQuery,
    GetLibraryUploadSongsQuery, GetLyricsIDQuery, GetLyricsQuery, GetMoodCategoriesQuery,
    GetMoodPlaylistsQuery, GetPlaylistDetailsQuery, GetPlaylistTracksQuery,
    GetSearchSuggestionsQuery, GetWatchPlaylistQuery, SearchQuery,
};

use crate::innertube::provider_adapter::BrowseDelegate;
use crate::providers::mappers::ytmapi as map;
use crate::providers::mappers::{deduplicate_playlists, deduplicate_tracks, parse_duration_str_ms};
use crate::unix_secs;

const TTL_EXPLORE_SECS: u64 = 30 * 60;
const TTL_SEARCH_SECS: u64 = 5 * 60;
const TTL_LIBRARY_SECS: u64 = 5 * 60;
const TTL_ENTITY_SECS: u64 = 60 * 60;

enum YtmapiClientState {
    Unauthenticated,
    Browser(YtMusic<BrowserToken>),
    OAuth(YtMusic<OAuthToken>),
}

/// Primary YouTube Music browse & authenticated library adapter backed by `ytmapi-rs`.
pub struct YtmapiBrowseAdapter {
    state: RwLock<YtmapiClientState>,
    cache: Arc<dyn CacheStore>,
}

impl YtmapiBrowseAdapter {
    pub fn new(cache: Arc<dyn CacheStore>) -> Self {
        Self {
            state: RwLock::new(YtmapiClientState::Unauthenticated),
            cache,
        }
    }

    /// Synchronizes Resonance's `AuthSession` into `ytmapi-rs` without any extra login flow.
    pub async fn sync_session(&self, session: Option<&AuthSession>) -> CoreResult<()> {
        match session {
            Some(s) if !s.cookie_header.trim().is_empty() => {
                let yt = YtMusic::from_cookie(s.cookie_header.trim())
                    .await
                    .map_err(|e| CoreError::Auth(AuthError::Keystore(e.to_string())))?;
                let mut guard = self.state.write().await;
                *guard = YtmapiClientState::Browser(yt);
                Ok(())
            }
            _ => {
                let mut guard = self.state.write().await;
                *guard = YtmapiClientState::Unauthenticated;
                Ok(())
            }
        }
    }

    /// Synchronizes an H3.1 TV `OAuthToken` into `ytmapi-rs`.
    pub async fn sync_oauth_token(&self, token: OAuthToken) {
        let yt = YtMusic::from_auth_token(token);
        let mut guard = self.state.write().await;
        *guard = YtmapiClientState::OAuth(yt);
    }

    pub async fn is_authenticated(&self) -> bool {
        let guard = self.state.read().await;
        matches!(
            *guard,
            YtmapiClientState::Browser(_) | YtmapiClientState::OAuth(_)
        )
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

    async fn run_query<Q>(
        &self,
        query: &Q,
    ) -> CoreResult<<Q as ytmapi_rs::query::Query<BrowserToken>>::Output>
    where
        Q: ytmapi_rs::query::Query<BrowserToken>
            + ytmapi_rs::query::Query<
                OAuthToken,
                Output = <Q as ytmapi_rs::query::Query<BrowserToken>>::Output,
            > + Sync,
    {
        let guard = self.state.read().await;
        let res = match &*guard {
            YtmapiClientState::Unauthenticated => {
                return Err(CoreError::Auth(AuthError::NotSignedIn));
            }
            YtmapiClientState::Browser(yt) => yt.query::<Q>(query).await,
            YtmapiClientState::OAuth(yt) => yt.query::<Q>(query).await,
        };
        res.map_err(|e| CoreError::Provider(ProviderError::Parse(e.to_string())))
    }
}

macro_rules! run_any_query {
    ($self:expr, $query:expr) => {
        $self.run_query($query).await
    };
}

macro_rules! run_logged_in_query {
    ($self:expr, $query:expr) => {
        $self.run_query($query).await
    };
}

fn text_runs_to_string(runs: &[TextRun]) -> String {
    runs.iter()
        .map(|r| match r {
            TextRun::Normal(t) | TextRun::Bold(t) => t.as_str(),
        })
        .collect::<String>()
}

fn map_table_list_song(s: TableListUploadSong) -> Track {
    Track {
        id: s.video_id.get_raw().to_owned(),
        title: s.title,
        artists: s
            .artists
            .into_iter()
            .map(|a| resonance_contract::ArtistRef {
                id: a.id.map(|id| id.get_raw().to_owned()),
                name: a.name,
            })
            .collect(),
        album: s.album.map(|a| resonance_contract::AlbumRef {
            id: Some(a.id.get_raw().to_owned()),
            title: a.name,
        }),
        duration_ms: parse_duration_str_ms(&s.duration),
        artwork_url: map::best_thumb(&s.thumbnails),
        is_explicit: false,
        is_liked: matches!(s.like_status, ytmapi_rs::common::LikeStatus::Liked),
        is_video: false,
    }
}

fn map_library_album_ref(a: SearchResultAlbum) -> LibraryCollectionRef {
    let id = a.album_id.get_raw().to_owned();
    let subtitle = if a.year.is_empty() {
        a.artist
    } else {
        format!("{} • {}", a.artist, a.year)
    };
    LibraryCollectionRef {
        browse_id: id,
        playlist_id: None,
        title: a.title,
        subtitle,
        artwork_url: map::best_thumb(&a.thumbnails),
        track_count: None,
    }
}

#[async_trait]
impl BrowseDelegate for YtmapiBrowseAdapter {
    async fn search(&self, query: &str) -> CoreResult<SearchResults> {
        self.search_categorized(query, SearchCategory::All, None)
            .await
    }

    async fn home(&self) -> CoreResult<Vec<HomeSection>> {
        // ytmapi-rs 0.3.x does not expose GetHomeQuery; HybridMusicProvider
        // transparently fails over to RustyPipeBrowseAdapter / InnerTubeClient.
        Err(CoreError::Provider(ProviderError::Parse(
            "ytmapi-rs 0.3 does not implement GetHomeQuery".into(),
        )))
    }

    async fn browse_collection(&self, browse_id: &str) -> CoreResult<Playlist> {
        let cache_key = format!("ytmapi:collection:{browse_id}");
        let (cached, is_fresh) = self.read_swr::<Playlist>(&cache_key);
        if is_fresh {
            if let Some(pl) = cached {
                return Ok(pl);
            }
        }

        let res: CoreResult<Playlist> =
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
                let pl_id = PlaylistID::from_raw(browse_id);
                let details_q = GetPlaylistDetailsQuery::new(pl_id.clone());
                let tracks_q = GetPlaylistTracksQuery::new(pl_id);
                let details_res = run_any_query!(self, &details_q);
                let tracks_res = run_any_query!(self, &tracks_q);
                match (details_res, tracks_res) {
                    (Ok(details), Ok(tracks)) => Ok(Playlist {
                        id: browse_id.to_owned(),
                        title: details.title,
                        subtitle: Some(details.author).filter(|a| !a.is_empty()),
                        artwork_url: map::best_thumb(&details.thumbnails),
                        tracks: map::map_playlist_items(tracks),
                    }),
                    (Err(_), Ok(tracks)) => Ok(Playlist {
                        id: browse_id.to_owned(),
                        title: browse_id.to_owned(),
                        subtitle: None,
                        artwork_url: None,
                        tracks: map::map_playlist_items(tracks),
                    }),
                    (Err(e), Err(_)) => Err(e),
                    (Ok(details), Err(_)) => Ok(Playlist {
                        id: browse_id.to_owned(),
                        title: details.title,
                        subtitle: Some(details.author).filter(|a| !a.is_empty()),
                        artwork_url: map::best_thumb(&details.thumbnails),
                        tracks: Vec::new(),
                    }),
                }
            };

        match res {
            Ok(pl) => {
                self.write_swr(&cache_key, &pl, TTL_ENTITY_SECS);
                Ok(pl)
            }
            Err(e) => cached.ok_or(e),
        }
    }

    async fn liked_songs(&self) -> CoreResult<Vec<Track>> {
        let query = GetPlaylistTracksQuery::new(PlaylistID::from_raw("LM"));
        let items = run_logged_in_query!(self, &query)?;
        Ok(deduplicate_tracks(map::map_playlist_items(items)))
    }

    async fn autoplay_next(&self, video_id: &str) -> CoreResult<Vec<Track>> {
        let query = GetWatchPlaylistQuery::new_from_video_id(VideoID::from_raw(video_id));
        let watch = run_any_query!(self, &query)?;
        Ok(deduplicate_tracks(map::map_watch_tracks(watch)))
    }

    async fn ytm_lyrics(&self, video_id: &str) -> CoreResult<Option<LyricsDocument>> {
        let id_query = GetLyricsIDQuery::new(VideoID::from_raw(video_id));
        let lyrics_id = match run_any_query!(self, &id_query) {
            Ok(id) => id,
            Err(_) => return Ok(None),
        };
        let lyrics_query = GetLyricsQuery::new(LyricsID::from_raw(lyrics_id.get_raw()));
        let lyrics: Lyrics = run_any_query!(self, &lyrics_query)?;
        Ok(Some(map::map_lyrics(video_id, lyrics)))
    }
}

#[async_trait]
impl CatalogExtProvider for YtmapiBrowseAdapter {
    async fn explore(&self) -> CoreResult<ExplorePage> {
        let cache_key = "ytmapi:explore:v1";
        let (cached, is_fresh) = self.read_swr::<ExplorePage>(cache_key);
        if is_fresh {
            if let Some(page) = cached {
                return Ok(page);
            }
        }

        let res = run_any_query!(self, &GetMoodCategoriesQuery);
        match res {
            Ok(sections) => {
                let page = ExplorePage {
                    new_releases: Vec::new(),
                    charts: Vec::new(),
                    moods_and_genres: map::map_mood_categories(sections),
                };
                self.write_swr(cache_key, &page, TTL_EXPLORE_SECS);
                Ok(page)
            }
            Err(e) => cached.ok_or(e),
        }
    }

    async fn mood_playlists(&self, params: &str) -> CoreResult<Vec<LibraryCollectionRef>> {
        let cache_key = format!("ytmapi:mood:{params}");
        let (cached, is_fresh) = self.read_swr::<Vec<LibraryCollectionRef>>(&cache_key);
        if is_fresh {
            if let Some(list) = cached {
                return Ok(list);
            }
        }

        let query = GetMoodPlaylistsQuery::new(MoodCategoryParams::from_raw(params));
        match run_any_query!(self, &query) {
            Ok(cats) => {
                let list = map::map_mood_playlists(cats);
                self.write_swr(&cache_key, &list, TTL_EXPLORE_SECS);
                Ok(list)
            }
            Err(e) => cached.ok_or(e),
        }
    }

    async fn search_categorized(
        &self,
        query: &str,
        category: SearchCategory,
        _continuation: Option<&str>,
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

        let cache_key = format!("ytmapi:search:{category:?}:{trimmed}");
        let (cached, is_fresh) = self.read_swr::<SearchResults>(&cache_key);
        if is_fresh {
            if let Some(res) = cached {
                return Ok(res);
            }
        }

        let res: CoreResult<SearchResults> = match category {
            SearchCategory::All => {
                let q = SearchQuery::new(trimmed);
                let raw = run_any_query!(self, &q)?;
                let mut tracks: Vec<Track> = raw
                    .songs
                    .into_iter()
                    .map(map::map_search_song)
                    .chain(raw.videos.into_iter().filter_map(map::map_search_video))
                    .collect();
                let mut albums: Vec<Playlist> = raw
                    .albums
                    .into_iter()
                    .map(map::map_search_album)
                    .chain(raw.artists.into_iter().map(map::map_search_artist))
                    .chain(
                        raw.featured_playlists
                            .into_iter()
                            .map(map::map_search_featured_playlist),
                    )
                    .chain(
                        raw.community_playlists
                            .into_iter()
                            .map(map::map_search_community_playlist),
                    )
                    .chain(raw.podcasts.into_iter().map(map::map_search_podcast))
                    .collect();
                tracks = deduplicate_tracks(tracks);
                albums = deduplicate_playlists(albums);
                Ok(SearchResults {
                    query: trimmed.to_owned(),
                    tracks,
                    albums,
                    continuation: None,
                })
            }
            SearchCategory::Songs => {
                let q = SearchQuery::new(trimmed).with_filter(SongsFilter);
                let songs = run_any_query!(self, &q)?;
                Ok(SearchResults {
                    query: trimmed.to_owned(),
                    tracks: deduplicate_tracks(
                        songs.into_iter().map(map::map_search_song).collect(),
                    ),
                    albums: Vec::new(),
                    continuation: None,
                })
            }
            SearchCategory::Albums => {
                let q = SearchQuery::new(trimmed).with_filter(AlbumsFilter);
                let albums = run_any_query!(self, &q)?;
                Ok(SearchResults {
                    query: trimmed.to_owned(),
                    tracks: Vec::new(),
                    albums: deduplicate_playlists(
                        albums.into_iter().map(map::map_search_album).collect(),
                    ),
                    continuation: None,
                })
            }
            SearchCategory::Artists => {
                let q = SearchQuery::new(trimmed).with_filter(ArtistsFilter);
                let artists = run_any_query!(self, &q)?;
                Ok(SearchResults {
                    query: trimmed.to_owned(),
                    tracks: Vec::new(),
                    albums: deduplicate_playlists(
                        artists.into_iter().map(map::map_search_artist).collect(),
                    ),
                    continuation: None,
                })
            }
            SearchCategory::Playlists => {
                let q = SearchQuery::new(trimmed).with_filter(CommunityPlaylistsFilter);
                let playlists = run_any_query!(self, &q)?;
                Ok(SearchResults {
                    query: trimmed.to_owned(),
                    tracks: Vec::new(),
                    albums: deduplicate_playlists(
                        playlists
                            .into_iter()
                            .map(map::map_search_playlist)
                            .collect(),
                    ),
                    continuation: None,
                })
            }
            SearchCategory::Podcasts => {
                let q = SearchQuery::new(trimmed).with_filter(PodcastsFilter);
                let podcasts = run_any_query!(self, &q)?;
                Ok(SearchResults {
                    query: trimmed.to_owned(),
                    tracks: Vec::new(),
                    albums: deduplicate_playlists(
                        podcasts.into_iter().map(map::map_search_podcast).collect(),
                    ),
                    continuation: None,
                })
            }
        };

        match res {
            Ok(sr) => {
                self.write_swr(&cache_key, &sr, TTL_SEARCH_SECS);
                Ok(sr)
            }
            Err(e) => cached.ok_or(e),
        }
    }

    async fn search_suggestions(&self, prefix: &str) -> CoreResult<Vec<String>> {
        let trimmed = prefix.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let q = GetSearchSuggestionsQuery::new(trimmed);
        let suggestions = run_any_query!(self, &q)?;
        Ok(suggestions
            .into_iter()
            .map(|s| text_runs_to_string(&s.runs))
            .filter(|s| !s.is_empty())
            .collect())
    }

    async fn fetch_library_snapshot(&self) -> CoreResult<LibrarySnapshot> {
        let cache_key = "ytmapi:library_snapshot:v1";
        let (cached, is_fresh) = self.read_swr::<LibrarySnapshot>(cache_key);
        if is_fresh {
            if let Some(snap) = cached {
                return Ok(snap);
            }
        }

        if !self.is_authenticated().await {
            return Err(CoreError::Auth(AuthError::NotSignedIn));
        }

        let guard = self.state.read().await;
        let liked_q = GetPlaylistTracksQuery::new(PlaylistID::from_raw("LM"));
        let playlists_q = GetLibraryPlaylistsQuery;
        let albums_q = GetLibraryAlbumsQuery::default();
        let artists_q = GetLibraryArtistsQuery::default();
        let history_q = GetHistoryQuery;
        let podcasts_q = GetLibraryPodcastsQuery::default();
        let uploads_q = GetLibraryUploadSongsQuery::default();

        let (
            liked_res,
            playlists_res,
            albums_res,
            artists_res,
            history_res,
            podcasts_res,
            uploads_res,
        ) = match &*guard {
            YtmapiClientState::Browser(yt) => tokio::join!(
                yt.query(liked_q),
                yt.query(playlists_q),
                yt.query(albums_q),
                yt.query(artists_q),
                yt.query(history_q),
                yt.query(podcasts_q),
                yt.query(uploads_q),
            ),
            YtmapiClientState::OAuth(yt) => tokio::join!(
                yt.query(liked_q),
                yt.query(playlists_q),
                yt.query(albums_q),
                yt.query(artists_q),
                yt.query(history_q),
                yt.query(podcasts_q),
                yt.query(uploads_q),
            ),
            YtmapiClientState::Unauthenticated => {
                return Err(CoreError::Auth(AuthError::NotSignedIn));
            }
        };

        let snapshot = LibrarySnapshot {
            liked_songs: liked_res
                .map(map::map_playlist_items)
                .map(deduplicate_tracks)
                .unwrap_or_default(),
            playlists: playlists_res
                .map(|v: Vec<ytmapi_rs::parse::LibraryPlaylist>| {
                    v.into_iter().map(map::map_library_playlist).collect()
                })
                .unwrap_or_default(),
            albums: albums_res
                .map(|v: Vec<SearchResultAlbum>| v.into_iter().map(map_library_album_ref).collect())
                .unwrap_or_default(),
            artists: artists_res
                .map(|v: Vec<ytmapi_rs::parse::LibraryArtist>| {
                    v.into_iter().map(map::map_library_artist).collect()
                })
                .unwrap_or_default(),
            history: history_res.map(map::map_history_items).unwrap_or_default(),
            podcasts: podcasts_res
                .map(|v: Vec<ytmapi_rs::parse::LibraryPodcast>| {
                    v.into_iter().map(map::map_library_podcast).collect()
                })
                .unwrap_or_default(),
            uploads: uploads_res
                .map(|v: Vec<TableListUploadSong>| v.into_iter().map(map_table_list_song).collect())
                .map(deduplicate_tracks)
                .unwrap_or_default(),
            fetched_at_unix_secs: unix_secs(),
        };

        self.write_swr(cache_key, &snapshot, TTL_LIBRARY_SECS);
        Ok(snapshot)
    }

    async fn fetch_artist(&self, channel_id: &str) -> CoreResult<ArtistDetail> {
        let cache_key = format!("ytmapi:artist:{channel_id}");
        let (cached, is_fresh) = self.read_swr::<ArtistDetail>(&cache_key);
        if is_fresh {
            if let Some(detail) = cached {
                return Ok(detail);
            }
        }

        let query = GetArtistQuery::new(ArtistChannelID::from_raw(channel_id));
        match run_any_query!(self, &query) {
            Ok(artist) => {
                let detail = map::map_artist_detail(artist);
                self.write_swr(&cache_key, &detail, TTL_ENTITY_SECS);
                Ok(detail)
            }
            Err(e) => cached.ok_or(e),
        }
    }

    async fn fetch_album(&self, browse_id: &str) -> CoreResult<AlbumDetail> {
        let cache_key = format!("ytmapi:album:{browse_id}");
        let (cached, is_fresh) = self.read_swr::<AlbumDetail>(&cache_key);
        if is_fresh {
            if let Some(detail) = cached {
                return Ok(detail);
            }
        }

        let query = GetAlbumQuery::new(AlbumID::from_raw(browse_id));
        match run_any_query!(self, &query) {
            Ok(album) => {
                let detail = map::map_album_detail(browse_id, album);
                self.write_swr(&cache_key, &detail, TTL_ENTITY_SECS);
                Ok(detail)
            }
            Err(e) => cached.ok_or(e),
        }
    }

    async fn create_playlist(
        &self,
        title: &str,
        description: &str,
        video_ids: &[String],
    ) -> CoreResult<String> {
        let desc = (!description.trim().is_empty()).then_some(description);
        let create_q = CreatePlaylistQuery::new(title, desc, PrivacyStatus::Private);
        let playlist_id = run_logged_in_query!(self, &create_q)?;
        let raw_id = playlist_id.get_raw().to_owned();
        if !video_ids.is_empty() {
            let vids: Vec<VideoID<'_>> = video_ids
                .iter()
                .map(|id| VideoID::from_raw(id.as_str()))
                .collect();
            let add_q = AddPlaylistItemsQuery::new_from_videos(
                playlist_id.into(),
                vids,
                DuplicateHandlingMode::ReturnError,
            );
            let _ = run_logged_in_query!(self, &add_q)?;
        }
        Ok(raw_id)
    }
}
