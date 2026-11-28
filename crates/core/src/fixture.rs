//! Deterministic `--fixture` QA dataset for Visual & Headless verification.
//!
//! Includes required torture-test items:
//! 1. A 70+ character track title (verifies clean truncation/ellipsis without clipping)
//! 2. CJK + emoji metadata (verifies UTF-8 & font fallback)
//! 3. A missing-artwork fallback item (verifies deterministic monogram artwork, zero blank boxes)
//! 4. Active synced karaoke lyrics with timed lines
//! 5. SponsorBlock `music_offtopic` intro segment

use async_trait::async_trait;
use resonance_contract::{
    AlbumRef, ArtistRef, AudioCodec, CoreResult, HomeItem, HomeSection, LyricLine, LyricSyncKind,
    LyricsDocument, LyricsProvider, MusicProvider, Playlist, SearchResults, SponsorBlockProvider,
    SponsorSegment, StreamInfo, Track,
};

pub fn fixture_tracks() -> Vec<Track> {
    vec![
        Track {
            id: "fix-long-70".into(),
            // Exactly 74 chars to torture-test single-line & two-line truncation
            title: "Midnight Reflections Across the Neon Boulevard of Eternal Synthesized Echo"
                .into(),
            artists: vec![
                ArtistRef {
                    id: Some("UC_ART_1".into()),
                    name: "Kaelen Vance".into(),
                },
                ArtistRef {
                    id: Some("UC_ART_2".into()),
                    name: "Solstice Collective".into(),
                },
            ],
            album: Some(AlbumRef {
                id: Some("MPRE_FIX_1".into()),
                title: "Nocturne Telemetry (Deluxe Edition)".into(),
            }),
            duration_ms: 244_000,
            artwork_url: None, // Triggers crisp deterministic monogram fallback
            is_explicit: false,
            is_liked: true,
            is_video: false,
        },
        Track {
            id: "fix-cjk-emoji".into(),
            title: "夜に駆ける 🌙✨ — Tokyo Midnight Synthwave (2026 Remaster)".into(),
            artists: vec![ArtistRef {
                id: Some("UC_ART_3".into()),
                name: "星野サクラ 🌸 / 都市の響き".into(),
            }],
            album: Some(AlbumRef {
                id: Some("MPRE_FIX_2".into()),
                title: "ネオン東京 EP 🎧".into(),
            }),
            duration_ms: 218_000,
            artwork_url: None,
            is_explicit: false,
            is_liked: true,
            is_video: true,
        },
        Track {
            id: "fix-missing-art".into(),
            title: "Amber Horizon (Acoustic Session)".into(),
            artists: vec![ArtistRef {
                id: Some("UC_ART_4".into()),
                name: "Elena Rostova".into(),
            }],
            album: Some(AlbumRef {
                id: Some("MPRE_FIX_3".into()),
                title: "Unplugged Tape IV".into(),
            }),
            duration_ms: 196_000,
            artwork_url: None,
            is_explicit: false,
            is_liked: false,
            is_video: false,
        },
        Track {
            id: "fix-deep-bass".into(),
            title: "Subterranean Pulse".into(),
            artists: vec![ArtistRef {
                id: Some("UC_ART_5".into()),
                name: "Monolith".into(),
            }],
            album: Some(AlbumRef {
                id: Some("MPRE_FIX_4".into()),
                title: "Concrete Frequencies".into(),
            }),
            duration_ms: 268_000,
            artwork_url: None,
            is_explicit: true,
            is_liked: false,
            is_video: false,
        },
    ]
}

pub fn fixture_lyrics(video_id: &str) -> LyricsDocument {
    LyricsDocument {
        video_id: video_id.to_owned(),
        provider: "LRCLIB (Synced)".to_owned(),
        sync_kind: LyricSyncKind::Line,
        lines: vec![
            LyricLine {
                start_ms: 0,
                end_ms: 12_000,
                text: "♪  Instrumental Intro  ♪".into(),
                words: Vec::new(),
                is_instrumental: true,
            },
            LyricLine {
                start_ms: 12_000,
                end_ms: 24_000,
                text: "Streetlights blur into copper and glass".into(),
                words: Vec::new(),
                is_instrumental: false,
            },
            LyricLine {
                start_ms: 24_000,
                end_ms: 36_000,
                text: "Every quiet signal echoes past the rain".into(),
                words: Vec::new(),
                is_instrumental: false,
            },
            LyricLine {
                start_ms: 36_000,
                end_ms: 50_000,
                text: "Hold the frequency until the skyline wakes".into(),
                words: Vec::new(),
                is_instrumental: false,
            },
            LyricLine {
                start_ms: 50_000,
                end_ms: 66_000,
                text: "夜明け前のネオンが静かに揺れる ✨".into(),
                words: Vec::new(),
                is_instrumental: false,
            },
            LyricLine {
                start_ms: 66_000,
                end_ms: 82_000,
                text: "Running through the static on a clear channel".into(),
                words: Vec::new(),
                is_instrumental: false,
            },
        ],
    }
}

pub fn fixture_home_sections() -> Vec<HomeSection> {
    let tracks = fixture_tracks();
    let mut listen_again_items: Vec<HomeItem> = tracks.iter().cloned().map(HomeItem::Track).collect();
    listen_again_items.push(HomeItem::Collection {
        browse_id: "MPRE_FIX_1".into(),
        title: "Nocturne Telemetry".into(),
        subtitle: "Album • Kaelen Vance".into(),
        artwork_url: None,
    });
    listen_again_items.push(HomeItem::Collection {
        browse_id: "MPRE_FIX_2".into(),
        title: "ネオン東京 EP 🎧".into(),
        subtitle: "EP • 星野サクラ".into(),
        artwork_url: None,
    });

    let song_mix_tracks: Vec<HomeItem> = vec![
        Track {
            id: "mix-maheroo".into(),
            title: "Maheroo Maheroo".into(),
            artists: vec![
                ArtistRef { id: Some("UC_ART_SHREYA".into()), name: "Shreya Ghoshal".into() },
                ArtistRef { id: Some("UC_ART_DARSHAN".into()), name: "Darshan Raval".into() },
            ],
            album: Some(AlbumRef { id: Some("MPRE_MIX_1".into()), title: "Super Nani".into() }),
            duration_ms: 274_000,
            artwork_url: None,
            is_explicit: false,
            is_liked: true,
            is_video: false,
        },
        Track {
            id: "mix-raataan".into(),
            title: "Raataan Lambiyan".into(),
            artists: vec![
                ArtistRef { id: Some("UC_ART_JUBIN".into()), name: "Jubin Nautiyal".into() },
                ArtistRef { id: Some("UC_ART_ASEES".into()), name: "Asees Kaur".into() },
            ],
            album: Some(AlbumRef { id: Some("MPRE_MIX_2".into()), title: "Shershaah".into() }),
            duration_ms: 230_000,
            artwork_url: None,
            is_explicit: false,
            is_liked: true,
            is_video: false,
        },
        Track {
            id: "mix-kesariya".into(),
            title: "Kesariya".into(),
            artists: vec![
                ArtistRef { id: Some("UC_ART_ARIJIT".into()), name: "Arijit Singh".into() },
                ArtistRef { id: Some("UC_ART_PRITAM".into()), name: "Pritam".into() },
            ],
            album: Some(AlbumRef { id: Some("MPRE_MIX_3".into()), title: "Brahmastra".into() }),
            duration_ms: 268_000,
            artwork_url: None,
            is_explicit: false,
            is_liked: false,
            is_video: false,
        },
        Track {
            id: "mix-chaleya".into(),
            title: "Chaleya".into(),
            artists: vec![
                ArtistRef { id: Some("UC_ART_ARIJIT".into()), name: "Arijit Singh".into() },
                ArtistRef { id: Some("UC_ART_SHILPA".into()), name: "Shilpa Rao".into() },
            ],
            album: Some(AlbumRef { id: Some("MPRE_MIX_4".into()), title: "Jawan".into() }),
            duration_ms: 200_000,
            artwork_url: None,
            is_explicit: false,
            is_liked: false,
            is_video: false,
        },
        Track {
            id: "mix-apna-bana".into(),
            title: "Apna Bana Le".into(),
            artists: vec![
                ArtistRef { id: Some("UC_ART_ARIJIT".into()), name: "Arijit Singh".into() },
                ArtistRef { id: Some("UC_ART_SACHIN".into()), name: "Sachin-Jigar".into() },
            ],
            album: Some(AlbumRef { id: Some("MPRE_MIX_5".into()), title: "Bhediya".into() }),
            duration_ms: 261_000,
            artwork_url: None,
            is_explicit: false,
            is_liked: true,
            is_video: false,
        },
        Track {
            id: "mix-tum-hi-ho".into(),
            title: "Tum Hi Ho".into(),
            artists: vec![ArtistRef { id: Some("UC_ART_ARIJIT".into()), name: "Arijit Singh".into() }],
            album: Some(AlbumRef { id: Some("MPRE_MIX_6".into()), title: "Aashiqui 2".into() }),
            duration_ms: 262_000,
            artwork_url: None,
            is_explicit: false,
            is_liked: true,
            is_video: false,
        },
    ]
    .into_iter()
    .map(HomeItem::Track)
    .collect();

    vec![
        HomeSection {
            id: "listen-again".into(),
            title: "Listen again".into(),
            items: listen_again_items,
        },
        HomeSection {
            id: "song-mixes".into(),
            title: "Song mixes".into(),
            items: song_mix_tracks,
        },
        HomeSection {
            id: "daily-discover".into(),
            title: "Your daily discover".into(),
            items: vec![
                HomeItem::Collection {
                    browse_id: "VLPL_DISCOVER_1".into(),
                    title: "Discover Mix".into(),
                    subtitle: "Playlist • YouTube Music".into(),
                    artwork_url: None,
                },
                HomeItem::Collection {
                    browse_id: "VLPL_DISCOVER_2".into(),
                    title: "New Release Mix".into(),
                    subtitle: "Playlist • Updated Fridays".into(),
                    artwork_url: None,
                },
                HomeItem::Collection {
                    browse_id: "VLPL_DISCOVER_3".into(),
                    title: "Replay Mix".into(),
                    subtitle: "Playlist • Your top tracks".into(),
                    artwork_url: None,
                },
                HomeItem::Collection {
                    browse_id: "VLPL_DISCOVER_4".into(),
                    title: "Supermix".into(),
                    subtitle: "Playlist • Mixed for you".into(),
                    artwork_url: None,
                },
                HomeItem::Collection {
                    browse_id: "VLPL_DISCOVER_5".into(),
                    title: "Chill Mix".into(),
                    subtitle: "Playlist • Lo-fi & Acoustic".into(),
                    artwork_url: None,
                },
            ],
        },
        HomeSection {
            id: "from-community".into(),
            title: "From the community".into(),
            items: vec![
                HomeItem::Collection {
                    browse_id: "VLPL_COMM_1".into(),
                    title: "Love and Heartbreak".into(),
                    subtitle: "YRF • 15 tracks".into(),
                    artwork_url: None,
                },
                HomeItem::Collection {
                    browse_id: "VLPL_COMM_2".into(),
                    title: "Adrenaline Boosters".into(),
                    subtitle: "Sandeep Chahar • 20 tracks".into(),
                    artwork_url: None,
                },
                HomeItem::Collection {
                    browse_id: "VLPL_COMM_3".into(),
                    title: "Indian Indie Essentials".into(),
                    subtitle: "Indie Radar • 32 tracks".into(),
                    artwork_url: None,
                },
                HomeItem::Collection {
                    browse_id: "VLPL_COMM_4".into(),
                    title: "Late Night Monsoon".into(),
                    subtitle: "Curated • 28 tracks".into(),
                    artwork_url: None,
                },
            ],
        },
        HomeSection {
            id: "new-releases".into(),
            title: "New releases".into(),
            items: vec![
                HomeItem::Collection {
                    browse_id: "MPRE_FIX_1".into(),
                    title: "Nocturne Telemetry".into(),
                    subtitle: "Album • Kaelen Vance".into(),
                    artwork_url: None,
                },
                HomeItem::Collection {
                    browse_id: "MPRE_FIX_2".into(),
                    title: "ネオン東京 EP 🎧".into(),
                    subtitle: "EP • 星野サクラ".into(),
                    artwork_url: None,
                },
                HomeItem::Collection {
                    browse_id: "MPRE_FIX_3".into(),
                    title: "Unplugged Tape IV".into(),
                    subtitle: "Album • Elena Rostova".into(),
                    artwork_url: None,
                },
                HomeItem::Collection {
                    browse_id: "MPRE_FIX_4".into(),
                    title: "Concrete Frequencies".into(),
                    subtitle: "Album • Monolith".into(),
                    artwork_url: None,
                },
            ],
        },
        HomeSection {
            id: "indian-pop".into(),
            title: "Indian pop".into(),
            items: vec![
                HomeItem::Collection {
                    browse_id: "VLPL_INDIE_1".into(),
                    title: "Bollywood Mush".into(),
                    subtitle: "Playlist • YouTube Music".into(),
                    artwork_url: None,
                },
                HomeItem::Collection {
                    browse_id: "VLPL_INDIE_2".into(),
                    title: "Desi Pop Anthems".into(),
                    subtitle: "Playlist • 40 tracks".into(),
                    artwork_url: None,
                },
                HomeItem::Collection {
                    browse_id: "VLPL_INDIE_3".into(),
                    title: "Acoustic Unplugged".into(),
                    subtitle: "Playlist • 24 tracks".into(),
                    artwork_url: None,
                },
            ],
        },
        HomeSection {
            id: "live-performances".into(),
            title: "Live performances".into(),
            items: vec![
                HomeItem::Collection {
                    browse_id: "VLPL_LIVE_1".into(),
                    title: "Coke Studio Sessions".into(),
                    subtitle: "Live • 18 videos".into(),
                    artwork_url: None,
                },
                HomeItem::Collection {
                    browse_id: "VLPL_LIVE_2".into(),
                    title: "Midnight Tiny Desk".into(),
                    subtitle: "Live • 14 videos".into(),
                    artwork_url: None,
                },
            ],
        },
        HomeSection {
            id: "recaps".into(),
            title: "Recaps".into(),
            items: vec![
                HomeItem::Collection {
                    browse_id: "VLPL_RECAP_1".into(),
                    title: "2026 Seasonal Recap".into(),
                    subtitle: "Auto playlist • 25 tracks".into(),
                    artwork_url: None,
                },
                HomeItem::Collection {
                    browse_id: "VLPL_RECAP_2".into(),
                    title: "Sounds from Shorts".into(),
                    subtitle: "Auto playlist • 16 tracks".into(),
                    artwork_url: None,
                },
            ],
        },
    ]
}

/// Deterministic offline `MusicProvider` + `LyricsProvider` + `SponsorBlockProvider` for `--fixture`.
pub struct FixtureProvider {
    pub stream_url: String,
}

impl FixtureProvider {
    pub fn new(stream_url: impl Into<String>) -> Self {
        Self {
            stream_url: stream_url.into(),
        }
    }
}

#[async_trait]
impl MusicProvider for FixtureProvider {
    async fn search(&self, query: &str) -> CoreResult<SearchResults> {
        let q = query.trim().to_lowercase();
        let tracks = fixture_tracks()
            .into_iter()
            .filter(|t| {
                q.is_empty()
                    || t.title.to_lowercase().contains(&q)
                    || t.artist_display().to_lowercase().contains(&q)
            })
            .collect();
        Ok(SearchResults {
            query: query.to_owned(),
            tracks,
            albums: Vec::new(),
            continuation: None,
        })
    }

    async fn home(&self) -> CoreResult<Vec<HomeSection>> {
        Ok(fixture_home_sections())
    }

    async fn browse_collection(&self, browse_id: &str) -> CoreResult<Playlist> {
        Ok(Playlist {
            id: browse_id.to_owned(),
            title: "Fixture Collection".into(),
            subtitle: Some("Deterministic QA Seed".into()),
            artwork_url: None,
            tracks: fixture_tracks(),
        })
    }

    async fn liked_songs(&self) -> CoreResult<Vec<Track>> {
        Ok(fixture_tracks()
            .into_iter()
            .filter(|t| t.is_liked)
            .collect())
    }

    async fn autoplay_next(&self, _video_id: &str) -> CoreResult<Vec<Track>> {
        Ok(fixture_tracks())
    }

    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo> {
        let dur = fixture_tracks()
            .into_iter()
            .find(|t| t.id == video_id)
            .map_or(210_000, |t| t.duration_ms);
        Ok(StreamInfo {
            video_id: video_id.to_owned(),
            url: self.stream_url.clone(),
            itag: 251,
            mime_type: "audio/webm; codecs=\"opus\"".into(),
            codec: AudioCodec::Opus,
            bitrate_kbps: 160,
            content_length: 512 * 1024,
            duration_ms: dur,
            loudness_db: Some(-1.2),
            resolved_by_client: "FIXTURE".into(),
            expires_at_unix_secs: crate::unix_secs() + 3600,
        })
    }

    async fn ytm_lyrics(&self, video_id: &str) -> CoreResult<Option<LyricsDocument>> {
        Ok(Some(fixture_lyrics(video_id)))
    }
}

#[async_trait]
impl LyricsProvider for FixtureProvider {
    async fn fetch_lyrics(&self, track: &Track) -> CoreResult<Option<LyricsDocument>> {
        Ok(Some(fixture_lyrics(&track.id)))
    }
}

#[async_trait]
impl SponsorBlockProvider for FixtureProvider {
    async fn fetch_segments(&self, video_id: &str) -> CoreResult<Vec<SponsorSegment>> {
        if video_id == "fix-cjk-emoji" {
            Ok(vec![SponsorSegment {
                start_secs: 0.0,
                end_secs: 14.0,
                video_duration_secs: 218.0,
            }])
        } else {
            Ok(Vec::new())
        }
    }
}
