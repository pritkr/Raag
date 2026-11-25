//! Pure mapping helpers from `ytmapi-rs` and `rustypipe` models to frozen
//! `crates/contract` domain types.

use std::collections::HashSet;

use resonance_contract::{
    AlbumDetail, AlbumRef, ArtistDetail, ArtistRef, HomeItem, HomeSection, LibraryCollectionRef,
    LyricLine, LyricSyncKind, LyricsDocument, MoodCategory, Playlist, Track,
};

/// Parses `"mm:ss"` or `"hh:mm:ss"` duration strings into milliseconds.
pub fn parse_duration_str_ms(raw: &str) -> u64 {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return 0;
    }
    let parts: Vec<u64> = trimmed
        .split(':')
        .filter_map(|p| p.trim().parse::<u64>().ok())
        .collect();
    let secs = match parts.as_slice() {
        [s] => *s,
        [m, s] => m.saturating_mul(60).saturating_add(*s),
        [h, m, s] => h
            .saturating_mul(3600)
            .saturating_add(m.saturating_mul(60))
            .saturating_add(*s),
        _ => 0,
    };
    secs.saturating_mul(1000)
}

/// Deduplicates tracks in order by `track.id`.
pub fn deduplicate_tracks(tracks: Vec<Track>) -> Vec<Track> {
    let mut seen = HashSet::with_capacity(tracks.len());
    let mut out = Vec::with_capacity(tracks.len());
    for t in tracks {
        if !t.id.is_empty() && seen.insert(t.id.clone()) {
            out.push(t);
        }
    }
    out
}

/// Deduplicates playlists/collections in order by `playlist.id`.
pub fn deduplicate_playlists(playlists: Vec<Playlist>) -> Vec<Playlist> {
    let mut seen = HashSet::with_capacity(playlists.len());
    let mut out = Vec::with_capacity(playlists.len());
    for p in playlists {
        if !p.id.is_empty() && seen.insert(p.id.clone()) {
            out.push(p);
        }
    }
    out
}

#[cfg(feature = "provider-ytmapi")]
pub mod ytmapi {
    use super::*;
    use ytmapi_rs::common::{Explicit, LikeStatus, Thumbnail, YoutubeID};
    use ytmapi_rs::parse::{
        BasicSearchResultCommunityPlaylist, GetAlbum, GetArtist, HistoryItem, LibraryArtist,
        LibraryPlaylist, LibraryPodcast, Lyrics, MoodCategorySection, MoodPlaylistCategory,
        PlaylistItem, SearchResultAlbum, SearchResultArtist, SearchResultCommunityPlaylist,
        SearchResultFeaturedPlaylist, SearchResultPlaylist, SearchResultPodcast, SearchResultSong,
        SearchResultVideo, WatchPlaylistTrack,
    };

    pub fn best_thumb(thumbs: &[Thumbnail]) -> Option<String> {
        thumbs
            .iter()
            .max_by_key(|t| t.width.saturating_mul(t.height))
            .map(|t| t.url.clone())
    }

    fn is_explicit(e: Explicit) -> bool {
        matches!(e, Explicit::IsExplicit)
    }

    fn is_liked(s: LikeStatus) -> bool {
        matches!(s, LikeStatus::Liked)
    }

    pub fn map_search_song(s: SearchResultSong) -> Track {
        Track {
            id: s.video_id.get_raw().to_owned(),
            title: s.title,
            artists: if s.artist.is_empty() {
                Vec::new()
            } else {
                vec![ArtistRef {
                    id: None,
                    name: s.artist,
                }]
            },
            album: s.album.map(|a| AlbumRef {
                id: Some(a.id.get_raw().to_owned()),
                title: a.name,
            }),
            duration_ms: parse_duration_str_ms(&s.duration),
            artwork_url: best_thumb(&s.thumbnails),
            is_explicit: is_explicit(s.explicit),
            is_liked: false,
            is_video: false,
        }
    }

    pub fn map_search_video(v: SearchResultVideo) -> Option<Track> {
        match v {
            SearchResultVideo::Video {
                title,
                channel_name,
                video_id,
                length,
                thumbnails,
                ..
            } => Some(Track {
                id: video_id.get_raw().to_owned(),
                title,
                artists: if channel_name.is_empty() {
                    Vec::new()
                } else {
                    vec![ArtistRef {
                        id: None,
                        name: channel_name,
                    }]
                },
                album: None,
                duration_ms: parse_duration_str_ms(&length),
                artwork_url: best_thumb(&thumbnails),
                is_explicit: false,
                is_liked: false,
                is_video: true,
            }),
            _ => None,
        }
    }

    pub fn map_search_album(a: SearchResultAlbum) -> Playlist {
        Playlist {
            id: a.album_id.get_raw().to_owned(),
            title: a.title,
            subtitle: Some(if a.year.is_empty() {
                a.artist
            } else {
                format!("{} • {}", a.artist, a.year)
            }),
            artwork_url: best_thumb(&a.thumbnails),
            tracks: Vec::new(),
        }
    }

    pub fn map_search_artist(a: SearchResultArtist) -> Playlist {
        Playlist {
            id: a.browse_id.get_raw().to_owned(),
            title: a.artist,
            subtitle: a.subscribers,
            artwork_url: best_thumb(&a.thumbnails),
            tracks: Vec::new(),
        }
    }

    pub fn map_search_community_playlist_item(p: SearchResultCommunityPlaylist) -> Playlist {
        Playlist {
            id: p.playlist_id.get_raw().to_owned(),
            title: p.title,
            subtitle: Some(format!("{} • {}", p.author, p.views)),
            artwork_url: best_thumb(&p.thumbnails),
            tracks: Vec::new(),
        }
    }

    pub fn map_search_community_playlist(p: BasicSearchResultCommunityPlaylist) -> Playlist {
        match p {
            BasicSearchResultCommunityPlaylist::Playlist(pl) => {
                map_search_community_playlist_item(pl)
            }
            BasicSearchResultCommunityPlaylist::Podcast(pod) => map_search_podcast(pod),
            _ => Playlist {
                id: String::new(),
                title: String::new(),
                subtitle: None,
                artwork_url: None,
                tracks: Vec::new(),
            },
        }
    }

    pub fn map_search_playlist(p: SearchResultPlaylist) -> Playlist {
        match p {
            SearchResultPlaylist::Featured(pl) => map_search_featured_playlist(pl),
            SearchResultPlaylist::Community(pl) => map_search_community_playlist_item(pl),
            SearchResultPlaylist::Podcast(pod) => map_search_podcast(pod),
            _ => Playlist {
                id: String::new(),
                title: String::new(),
                subtitle: None,
                artwork_url: None,
                tracks: Vec::new(),
            },
        }
    }

    pub fn map_search_featured_playlist(p: SearchResultFeaturedPlaylist) -> Playlist {
        Playlist {
            id: p.playlist_id.get_raw().to_owned(),
            title: p.title,
            subtitle: Some(format!("{} • {} songs", p.author, p.songs)),
            artwork_url: best_thumb(&p.thumbnails),
            tracks: Vec::new(),
        }
    }

    pub fn map_search_podcast(p: SearchResultPodcast) -> Playlist {
        Playlist {
            id: p.podcast_id.get_raw().to_owned(),
            title: p.title,
            subtitle: Some(p.publisher),
            artwork_url: best_thumb(&p.thumbnails),
            tracks: Vec::new(),
        }
    }

    pub fn map_album_detail(browse_id: &str, album: GetAlbum) -> AlbumDetail {
        let artwork = best_thumb(&album.thumbnails);
        let artists: Vec<ArtistRef> = album
            .artists
            .into_iter()
            .map(|a| ArtistRef {
                id: a.id.map(|id| id.get_raw().to_owned()),
                name: a.name,
            })
            .collect();
        let album_ref = AlbumRef {
            id: Some(browse_id.to_owned()),
            title: album.title.clone(),
        };
        let mut total_duration_ms = 0u64;
        let tracks = album
            .tracks
            .into_iter()
            .map(|t| {
                let dur = parse_duration_str_ms(&t.duration);
                total_duration_ms = total_duration_ms.saturating_add(dur);
                Track {
                    id: t.video_id.get_raw().to_owned(),
                    title: t.title,
                    artists: artists.clone(),
                    album: Some(album_ref.clone()),
                    duration_ms: dur,
                    artwork_url: artwork.clone(),
                    is_explicit: is_explicit(t.explicit),
                    is_liked: is_liked(t.like_status),
                    is_video: false,
                }
            })
            .collect();

        AlbumDetail {
            browse_id: browse_id.to_owned(),
            audio_playlist_id: album.audio_playlist_id.map(|id| id.get_raw().to_owned()),
            title: album.title,
            artists,
            release_year: (!album.year.is_empty()).then_some(album.year),
            release_type: format!("{:?}", album.category),
            description: album.description,
            artwork_url: artwork,
            total_duration_ms,
            tracks,
            other_versions: Vec::new(),
        }
    }

    pub fn map_artist_detail(artist: GetArtist) -> ArtistDetail {
        let artwork = best_thumb(&artist.thumbnails);
        let top_songs = artist
            .top_releases
            .songs
            .map(|s| {
                s.results
                    .into_iter()
                    .map(|song| Track {
                        id: song.video_id.get_raw().to_owned(),
                        title: song.title,
                        artists: song
                            .artists
                            .into_iter()
                            .map(|a| ArtistRef {
                                id: a.id.map(|id| id.get_raw().to_owned()),
                                name: a.name,
                            })
                            .collect(),
                        album: Some(AlbumRef {
                            id: Some(song.album.id.get_raw().to_owned()),
                            title: song.album.name,
                        }),
                        duration_ms: 0,
                        artwork_url: artwork.clone(),
                        is_explicit: is_explicit(song.explicit),
                        is_liked: is_liked(song.like_status),
                        is_video: false,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let albums = artist
            .top_releases
            .albums
            .map(|al| {
                al.results
                    .into_iter()
                    .map(|a| LibraryCollectionRef {
                        browse_id: a.album_id.get_raw().to_owned(),
                        playlist_id: None,
                        title: a.title,
                        subtitle: a.year,
                        artwork_url: best_thumb(&a.thumbnails),
                        track_count: None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let singles = artist
            .top_releases
            .singles
            .map(|sg| {
                sg.results
                    .into_iter()
                    .map(|s| LibraryCollectionRef {
                        browse_id: s.album_id.get_raw().to_owned(),
                        playlist_id: None,
                        title: s.title,
                        subtitle: s.year,
                        artwork_url: best_thumb(&s.thumbnails),
                        track_count: None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let related_artists = artist
            .top_releases
            .related
            .map(|rel| {
                rel.results
                    .into_iter()
                    .map(|r| LibraryCollectionRef {
                        browse_id: r.browse_id.get_raw().to_owned(),
                        playlist_id: None,
                        title: r.title,
                        subtitle: r.subscribers,
                        artwork_url: None,
                        track_count: None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        ArtistDetail {
            channel_id: artist.channel_id.get_raw().to_owned(),
            name: artist.name,
            description: artist.description,
            subscribers: artist.subscribers,
            is_subscribed: artist.subscribed,
            artwork_url: artwork,
            radio_playlist_id: artist.radio_id,
            shuffle_playlist_id: artist.shuffle_id,
            top_songs,
            albums,
            singles,
            related_artists,
        }
    }

    pub fn map_playlist_items(items: Vec<PlaylistItem>) -> Vec<Track> {
        items
            .into_iter()
            .filter_map(|item| match item {
                PlaylistItem::Song(s) => Some(Track {
                    id: s.video_id.get_raw().to_owned(),
                    title: s.title,
                    artists: s
                        .artists
                        .into_iter()
                        .map(|a| ArtistRef {
                            id: a.id.map(|id| id.get_raw().to_owned()),
                            name: a.name,
                        })
                        .collect(),
                    album: Some(AlbumRef {
                        id: Some(s.album.id.get_raw().to_owned()),
                        title: s.album.name,
                    }),
                    duration_ms: parse_duration_str_ms(&s.duration),
                    artwork_url: best_thumb(&s.thumbnails),
                    is_explicit: is_explicit(s.explicit),
                    is_liked: is_liked(s.like_status),
                    is_video: false,
                }),
                PlaylistItem::Video(v) => Some(Track {
                    id: v.video_id.get_raw().to_owned(),
                    title: v.title,
                    artists: if v.channel_name.is_empty() {
                        Vec::new()
                    } else {
                        vec![ArtistRef {
                            id: Some(v.channel_id.get_raw().to_owned()),
                            name: v.channel_name,
                        }]
                    },
                    album: None,
                    duration_ms: parse_duration_str_ms(&v.duration),
                    artwork_url: best_thumb(&v.thumbnails),
                    is_explicit: false,
                    is_liked: is_liked(v.like_status),
                    is_video: true,
                }),
                _ => None,
            })
            .collect()
    }

    pub fn map_watch_tracks(tracks: Vec<WatchPlaylistTrack>) -> Vec<Track> {
        tracks
            .into_iter()
            .map(|t| Track {
                id: t.video_id.get_raw().to_owned(),
                title: t.title,
                artists: if t.author.is_empty() {
                    Vec::new()
                } else {
                    vec![ArtistRef {
                        id: None,
                        name: t.author,
                    }]
                },
                album: None,
                duration_ms: parse_duration_str_ms(&t.duration),
                artwork_url: best_thumb(&t.thumbnails),
                is_explicit: false,
                is_liked: false,
                is_video: false,
            })
            .collect()
    }

    pub fn map_mood_categories(sections: Vec<MoodCategorySection>) -> Vec<MoodCategory> {
        let mut out = Vec::new();
        for sec in sections {
            for cat in sec.mood_categories {
                out.push(MoodCategory {
                    title: cat.title,
                    params: cat.params.get_raw().to_owned(),
                    accent_hint: Some(sec.section_name.clone()),
                });
            }
        }
        out
    }

    pub fn map_mood_playlists(cats: Vec<MoodPlaylistCategory>) -> Vec<LibraryCollectionRef> {
        let mut out = Vec::new();
        for cat in cats {
            for pl in cat.playlists {
                let id = pl.playlist_id.get_raw().to_owned();
                out.push(LibraryCollectionRef {
                    browse_id: id.clone(),
                    playlist_id: Some(id),
                    title: pl.title,
                    subtitle: cat.category_name.clone(),
                    artwork_url: best_thumb(&pl.thumbnails),
                    track_count: None,
                });
            }
        }
        out
    }

    pub fn map_lyrics(video_id: &str, l: Lyrics) -> LyricsDocument {
        let lines = l
            .lyrics
            .lines()
            .map(|line| LyricLine {
                start_ms: 0,
                end_ms: 0,
                text: line.to_owned(),
                words: Vec::new(),
                is_instrumental: line.trim().is_empty(),
            })
            .collect();
        LyricsDocument {
            video_id: video_id.to_owned(),
            provider: "YouTube Music (ytmapi-rs)".to_owned(),
            sync_kind: LyricSyncKind::Plain,
            lines,
        }
    }

    pub fn map_library_playlist(p: LibraryPlaylist) -> LibraryCollectionRef {
        let id = p.playlist_id.get_raw().to_owned();
        LibraryCollectionRef {
            browse_id: id.clone(),
            playlist_id: Some(id),
            title: p.title,
            subtitle: format!("{} • {}", p.author, p.tracks),
            artwork_url: best_thumb(&p.thumbnails),
            track_count: p
                .tracks
                .split_whitespace()
                .next()
                .and_then(|n| n.parse().ok()),
        }
    }

    pub fn map_library_artist(a: LibraryArtist) -> LibraryCollectionRef {
        LibraryCollectionRef {
            browse_id: a.channel_id.get_raw().to_owned(),
            playlist_id: None,
            title: a.artist,
            subtitle: a.byline,
            artwork_url: None,
            track_count: None,
        }
    }

    pub fn map_library_podcast(p: LibraryPodcast) -> LibraryCollectionRef {
        LibraryCollectionRef {
            browse_id: p.podcast_id.get_raw().to_owned(),
            playlist_id: None,
            title: p.title,
            subtitle: p
                .channels
                .into_iter()
                .map(|c| c.name)
                .collect::<Vec<_>>()
                .join(", "),
            artwork_url: best_thumb(&p.thumbnails),
            track_count: None,
        }
    }

    pub fn map_history_items(periods: Vec<ytmapi_rs::parse::HistoryPeriod>) -> Vec<Track> {
        let mut out = Vec::new();
        for period in periods {
            for item in period.items {
                match item {
                    HistoryItem::Song(s) => out.push(Track {
                        id: s.video_id.get_raw().to_owned(),
                        title: s.title,
                        artists: s
                            .artists
                            .into_iter()
                            .map(|a| ArtistRef {
                                id: a.id.map(|id| id.get_raw().to_owned()),
                                name: a.name,
                            })
                            .collect(),
                        album: Some(AlbumRef {
                            id: Some(s.album.id.get_raw().to_owned()),
                            title: s.album.name,
                        }),
                        duration_ms: parse_duration_str_ms(&s.duration),
                        artwork_url: best_thumb(&s.thumbnails),
                        is_explicit: is_explicit(s.explicit),
                        is_liked: is_liked(s.like_status),
                        is_video: false,
                    }),
                    HistoryItem::Video(v) => out.push(Track {
                        id: v.video_id.get_raw().to_owned(),
                        title: v.title,
                        artists: if v.channel_name.is_empty() {
                            Vec::new()
                        } else {
                            vec![ArtistRef {
                                id: Some(v.channel_id.get_raw().to_owned()),
                                name: v.channel_name,
                            }]
                        },
                        album: None,
                        duration_ms: parse_duration_str_ms(&v.duration),
                        artwork_url: best_thumb(&v.thumbnails),
                        is_explicit: false,
                        is_liked: is_liked(v.like_status),
                        is_video: true,
                    }),
                    _ => {}
                }
            }
        }
        deduplicate_tracks(out)
    }
}

#[cfg(feature = "provider-rustypipe")]
pub mod rustypipe_map {
    use super::*;
    use rustypipe::model::{
        AlbumItem, ArtistItem, MusicAlbum, MusicArtist, MusicCharts, MusicPlaylist,
        MusicPlaylistItem, Thumbnail, TrackItem,
    };

    pub fn best_thumb(thumbs: &[Thumbnail]) -> Option<String> {
        thumbs
            .iter()
            .max_by_key(|t| t.width.saturating_mul(t.height))
            .map(|t| t.url.clone())
    }

    pub fn map_track(t: TrackItem) -> Track {
        Track {
            id: t.id,
            title: t.name,
            artists: t
                .artists
                .into_iter()
                .map(|a| ArtistRef {
                    id: a.id,
                    name: a.name,
                })
                .collect(),
            album: t.album.map(|a| AlbumRef {
                id: Some(a.id),
                title: a.name,
            }),
            duration_ms: u64::from(t.duration.unwrap_or(0)).saturating_mul(1000),
            artwork_url: best_thumb(&t.cover),
            is_explicit: false,
            is_liked: false,
            is_video: t.track_type == rustypipe::model::TrackType::Video,
        }
    }

    pub fn map_album_item(a: AlbumItem) -> Playlist {
        let artists = a
            .artists
            .iter()
            .map(|ar| ar.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let subtitle = match (artists.is_empty(), a.year) {
            (false, Some(y)) => Some(format!("{artists} • {y}")),
            (false, None) => Some(artists),
            (true, Some(y)) => Some(y.to_string()),
            (true, None) => None,
        };
        Playlist {
            id: a.id,
            title: a.name,
            subtitle,
            artwork_url: best_thumb(&a.cover),
            tracks: Vec::new(),
        }
    }

    pub fn map_playlist_item(p: MusicPlaylistItem) -> Playlist {
        let subtitle = p
            .channel
            .map(|c| c.name)
            .or_else(|| p.track_count.map(|n| format!("{n} tracks")));
        Playlist {
            id: p.id,
            title: p.name,
            subtitle,
            artwork_url: best_thumb(&p.thumbnail),
            tracks: Vec::new(),
        }
    }

    pub fn map_artist_item(a: ArtistItem) -> Playlist {
        Playlist {
            id: a.id,
            title: a.name,
            subtitle: a.subscriber_count.map(|n| format!("{n} subscribers")),
            artwork_url: best_thumb(&a.avatar),
            tracks: Vec::new(),
        }
    }

    pub fn map_music_playlist(pl: MusicPlaylist) -> Playlist {
        let subtitle = pl
            .channel
            .map(|c| c.name)
            .or_else(|| pl.track_count.map(|n| format!("{n} tracks")));
        Playlist {
            id: pl.id,
            title: pl.name,
            subtitle,
            artwork_url: best_thumb(&pl.thumbnail),
            tracks: pl.tracks.items.into_iter().map(map_track).collect(),
        }
    }

    pub fn map_music_album(alb: MusicAlbum) -> AlbumDetail {
        let artwork = best_thumb(&alb.cover);
        let tracks: Vec<Track> = alb.tracks.into_iter().map(map_track).collect();
        let total_duration_ms = tracks.iter().map(|t| t.duration_ms).sum();
        AlbumDetail {
            browse_id: alb.id,
            audio_playlist_id: alb.playlist_id,
            title: alb.name,
            artists: alb
                .artists
                .into_iter()
                .map(|a| ArtistRef {
                    id: a.id,
                    name: a.name,
                })
                .collect(),
            release_year: alb.year.map(|y| y.to_string()),
            release_type: format!("{:?}", alb.album_type),
            description: alb.description.map(|d| {
                d.0.into_iter()
                    .filter_map(|c| match c {
                        rustypipe::model::richtext::TextComponent::Text { text, .. } => Some(text),
                        _ => None,
                    })
                    .collect::<String>()
            }),
            artwork_url: artwork,
            total_duration_ms,
            tracks,
            other_versions: alb
                .variants
                .into_iter()
                .map(|v| LibraryCollectionRef {
                    browse_id: v.id,
                    playlist_id: None,
                    title: v.name,
                    subtitle: v.year.map(|y| y.to_string()).unwrap_or_default(),
                    artwork_url: best_thumb(&v.cover),
                    track_count: None,
                })
                .collect(),
        }
    }

    pub fn map_music_artist(art: MusicArtist) -> ArtistDetail {
        ArtistDetail {
            channel_id: art.id,
            name: art.name,
            description: art.description,
            subscribers: art.subscriber_count.map(|n| format!("{n} subscribers")),
            is_subscribed: false,
            artwork_url: best_thumb(&art.header_image),
            radio_playlist_id: None,
            shuffle_playlist_id: None,
            top_songs: art.tracks.into_iter().map(map_track).collect(),
            albums: art
                .albums
                .into_iter()
                .map(|a| LibraryCollectionRef {
                    browse_id: a.id,
                    playlist_id: None,
                    title: a.name,
                    subtitle: a.year.map(|y| y.to_string()).unwrap_or_default(),
                    artwork_url: best_thumb(&a.cover),
                    track_count: None,
                })
                .collect(),
            singles: Vec::new(),
            related_artists: art
                .similar_artists
                .into_iter()
                .map(|r| LibraryCollectionRef {
                    browse_id: r.id,
                    playlist_id: None,
                    title: r.name,
                    subtitle: r
                        .subscriber_count
                        .map(|n| format!("{n} subscribers"))
                        .unwrap_or_default(),
                    artwork_url: best_thumb(&r.avatar),
                    track_count: None,
                })
                .collect(),
        }
    }

    pub fn map_charts(charts: MusicCharts) -> Vec<HomeSection> {
        let mut sections = Vec::new();
        if !charts.trending_tracks.is_empty() {
            sections.push(HomeSection {
                id: "charts-trending".to_owned(),
                title: "Trending".to_owned(),
                items: charts
                    .trending_tracks
                    .into_iter()
                    .map(|t| HomeItem::Track(map_track(t)))
                    .collect(),
            });
        }
        if !charts.top_tracks.is_empty() {
            sections.push(HomeSection {
                id: "charts-top-songs".to_owned(),
                title: "Top Songs".to_owned(),
                items: charts
                    .top_tracks
                    .into_iter()
                    .map(|t| HomeItem::Track(map_track(t)))
                    .collect(),
            });
        }
        if !charts.playlists.is_empty() {
            sections.push(HomeSection {
                id: "charts-playlists".to_owned(),
                title: "Chart Playlists".to_owned(),
                items: charts
                    .playlists
                    .into_iter()
                    .map(|p| HomeItem::Collection {
                        browse_id: p.id,
                        title: p.name,
                        subtitle: p
                            .channel
                            .map(|c| c.name)
                            .unwrap_or_else(|| "Chart".to_owned()),
                        artwork_url: best_thumb(&p.thumbnail),
                    })
                    .collect(),
            });
        }
        sections
    }
}
