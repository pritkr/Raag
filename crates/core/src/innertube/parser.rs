//! Pure, deterministic InnerTube JSON parsers for `WEB_REMIX`, `VISIONOS`, and `ANDROID_VR`.
//!
//! Zero network I/O here — easily tested with real YouTube Music JSON snapshots.

use resonance_contract::{
    AlbumRef, ArtistRef, AudioCodec, CoreError, CoreResult, HomeItem, HomeSection, LyricLine,
    LyricSyncKind, LyricsDocument, Playlist, ProviderError, SearchResults, StreamInfo, Track,
};
use serde_json::Value;

/// Extracts `responseContext.visitorData` from any InnerTube response.
pub fn extract_visitor_data(resp: &Value) -> Option<String> {
    resp.pointer("/responseContext/visitorData")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Parses duration strings like `"3:45"` or `"1:02:10"` into milliseconds.
pub fn parse_duration_text(text: &str) -> u64 {
    let parts: Vec<u64> = text
        .trim()
        .split(':')
        .filter_map(|p| p.trim().parse::<u64>().ok())
        .collect();
    let secs = match parts.as_slice() {
        [m, s] => m * 60 + s,
        [h, m, s] => h * 3600 + m * 60 + s,
        [s] => *s,
        _ => 0,
    };
    secs * 1000
}

fn join_runs(v: &Value) -> String {
    v.get("runs")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(|r| r.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .or_else(|| v.get("simpleText").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default()
}

fn best_thumbnail(v: &Value) -> Option<String> {
    let thumbs = v
        .pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")
        .or_else(|| v.pointer("/thumbnail/thumbnails"))
        .and_then(Value::as_array)?;
    thumbs
        .iter()
        .max_by_key(|t| t.get("width").and_then(Value::as_u64).unwrap_or(0))
        .and_then(|t| t.get("url").and_then(Value::as_str))
        .map(str::to_owned)
}

fn parse_music_responsive_item(item: &Value) -> Option<Track> {
    let r = item.get("musicResponsiveListItemRenderer").unwrap_or(item);
    let video_id = r
        .pointer("/playlistItemData/videoId")
        .or_else(|| {
            r.pointer(
                "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId",
            )
        })
        .or_else(|| {
            r.pointer(
                "/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text/runs/0/navigationEndpoint/watchEndpoint/videoId",
            )
        })
        .and_then(Value::as_str)?
        .to_owned();

    let flex = r.get("flexColumns").and_then(Value::as_array)?;
    let title = flex
        .first()
        .and_then(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text"))
        .map(join_runs)
        .filter(|s| !s.is_empty())?;

    let mut artists = Vec::new();
    let mut album = None;
    let mut duration_ms = 0u64;

    if let Some(runs) = flex
        .get(1)
        .and_then(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs"))
        .and_then(Value::as_array)
    {
        for run in runs {
            let text = run.get("text").and_then(Value::as_str).unwrap_or("").trim();
            if text.is_empty() || text == "•" || text == "·" || text == "&" || text == "," {
                continue;
            }
            if let Some(browse_id) = run
                .pointer("/navigationEndpoint/browseEndpoint/browseId")
                .and_then(Value::as_str)
            {
                if browse_id.starts_with("UC") {
                    artists.push(ArtistRef {
                        id: Some(browse_id.to_owned()),
                        name: text.to_owned(),
                    });
                } else if browse_id.starts_with("MPRE") || browse_id.starts_with("OLAK") {
                    album = Some(AlbumRef {
                        id: Some(browse_id.to_owned()),
                        title: text.to_owned(),
                    });
                }
            } else if text.contains(':') && text.chars().all(|c| c.is_ascii_digit() || c == ':') {
                duration_ms = parse_duration_text(text);
            } else if artists.is_empty()
                && !matches!(
                    text.to_ascii_lowercase().as_str(),
                    "song" | "video" | "single" | "album" | "ep"
                )
            {
                artists.push(ArtistRef {
                    id: None,
                    name: text.to_owned(),
                });
            }
        }
    }

    if duration_ms == 0
        && let Some(fixed) = r.get("fixedColumns").and_then(Value::as_array)
    {
        for col in fixed {
            let t = col
                .pointer("/musicResponsiveListItemFixedColumnRenderer/text")
                .map(join_runs)
                .unwrap_or_default();
            if t.contains(':') {
                duration_ms = parse_duration_text(&t);
            }
        }
    }

    let is_explicit = r
        .get("badges")
        .and_then(Value::as_array)
        .is_some_and(|badges| {
            badges.iter().any(|b| {
                b.pointer("/musicInlineBadgeRenderer/icon/iconType")
                    .and_then(Value::as_str)
                    == Some("MUSIC_EXPLICIT_BADGE")
            })
        });

    Some(Track {
        id: video_id,
        title,
        artists,
        album,
        duration_ms,
        artwork_url: best_thumbnail(r),
        is_explicit,
        is_liked: false,
        is_video: false,
    })
}

/// Parses `WEB_REMIX` `/youtubei/v1/search` JSON response.
pub fn parse_search_response(query: &str, resp: &Value) -> CoreResult<SearchResults> {
    let mut tracks = Vec::new();
    let mut albums = Vec::new();
    let mut continuation = None;

    let tabs = resp
        .pointer("/contents/tabbedSearchResultsRenderer/tabs")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    for tab in tabs {
        let sections = tab
            .pointer("/tabRenderer/content/sectionListRenderer/contents")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();

        for section in sections {
            if let Some(shelf) = section.get("musicShelfRenderer") {
                if continuation.is_none() {
                    continuation = shelf
                        .pointer("/continuations/0/nextContinuationData/continuation")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                let items = shelf
                    .get("contents")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                for item in items {
                    if let Some(track) = parse_music_responsive_item(item) {
                        tracks.push(track);
                    } else if let Some(r) = item.get("musicResponsiveListItemRenderer")
                        && let Some(browse_id) = r
                            .pointer("/navigationEndpoint/browseEndpoint/browseId")
                            .and_then(Value::as_str)
                    {
                        let title = r
                            .pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text")
                            .map(join_runs)
                            .unwrap_or_default();
                        let subtitle = r
                            .pointer("/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text")
                            .map(join_runs);
                        if !title.is_empty() {
                            albums.push(Playlist {
                                id: browse_id.to_owned(),
                                title,
                                subtitle,
                                artwork_url: best_thumbnail(r),
                                tracks: Vec::new(),
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(SearchResults {
        query: query.to_owned(),
        tracks,
        albums,
        continuation,
    })
}

/// Parses `WEB_REMIX` `/youtubei/v1/browse` (`FEmusic_home`) into `HomeSection`s.
pub fn parse_home_response(resp: &Value) -> CoreResult<Vec<HomeSection>> {
    let mut sections_out = Vec::new();
    let contents = resp
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    for (idx, sec) in contents.iter().enumerate() {
        let Some(carousel) = sec.get("musicCarouselShelfRenderer") else {
            continue;
        };
        let title = carousel
            .pointer("/header/musicCarouselShelfBasicHeaderRenderer/title")
            .map(join_runs)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("Section {}", idx + 1));

        let mut items = Vec::new();
        for raw in carousel
            .get("contents")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            if let Some(track) = parse_music_responsive_item(raw) {
                items.push(HomeItem::Track(track));
            } else if let Some(two_row) = raw.get("musicTwoRowItemRenderer") {
                let item_title = two_row.get("title").map(join_runs).unwrap_or_default();
                let subtitle = two_row.get("subtitle").map(join_runs).unwrap_or_default();
                let artwork_url = best_thumbnail(two_row);
                if let Some(video_id) = two_row
                    .pointer("/navigationEndpoint/watchEndpoint/videoId")
                    .and_then(Value::as_str)
                {
                    items.push(HomeItem::Track(Track {
                        id: video_id.to_owned(),
                        title: item_title,
                        artists: vec![ArtistRef {
                            id: None,
                            name: subtitle,
                        }],
                        album: None,
                        duration_ms: 0,
                        artwork_url,
                        is_explicit: false,
                        is_liked: false,
                        is_video: false,
                    }));
                } else if let Some(browse_id) = two_row
                    .pointer("/navigationEndpoint/browseEndpoint/browseId")
                    .and_then(Value::as_str)
                {
                    items.push(HomeItem::Collection {
                        browse_id: browse_id.to_owned(),
                        title: item_title,
                        subtitle,
                        artwork_url,
                    });
                }
            }
        }
        if !items.is_empty() {
            sections_out.push(HomeSection {
                id: format!("home-{idx}"),
                title,
                items,
            });
        }
    }
    Ok(sections_out)
}

/// Parses `WEB_REMIX` `/youtubei/v1/browse` for an album or playlist (`MPRE...`, `VL...`, `FEmusic_liked_videos`).
pub fn parse_collection_response(browse_id: &str, resp: &Value) -> CoreResult<Playlist> {
    let header = resp
        .pointer("/header/musicDetailHeaderRenderer")
        .or_else(|| resp.pointer("/header/musicResponsiveHeaderRenderer"))
        .or_else(|| {
            resp.pointer(
                "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer",
            )
        });

    let title = header
        .and_then(|h| h.get("title"))
        .map(join_runs)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| browse_id.to_owned());
    let subtitle = header.and_then(|h| h.get("subtitle")).map(join_runs);
    let artwork_url = header.and_then(best_thumbnail);

    let mut tracks = Vec::new();
    let shelf_paths = [
        "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents",
        "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents",
    ];
    for path in shelf_paths {
        if let Some(sections) = resp.pointer(path).and_then(Value::as_array) {
            for sec in sections {
                let items = sec
                    .pointer("/musicShelfRenderer/contents")
                    .or_else(|| sec.pointer("/musicPlaylistShelfRenderer/contents"))
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                for item in items {
                    if let Some(track) = parse_music_responsive_item(item) {
                        tracks.push(track);
                    }
                }
            }
        }
    }

    Ok(Playlist {
        id: browse_id.to_owned(),
        title,
        subtitle,
        artwork_url,
        tracks,
    })
}

/// Parses `WEB_REMIX` `/youtubei/v1/next` autoplay radio queue tracks.
pub fn parse_next_queue_response(resp: &Value) -> CoreResult<Vec<Track>> {
    let mut tracks = Vec::new();
    let items = resp
        .pointer("/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs/0/tabRenderer/content/musicQueueRenderer/content/playlistPanelRenderer/contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    for entry in items {
        let Some(p) = entry.get("playlistPanelVideoRenderer") else {
            continue;
        };
        let Some(video_id) = p.get("videoId").and_then(Value::as_str) else {
            continue;
        };
        let title = p.get("title").map(join_runs).unwrap_or_default();
        let artist = p
            .get("shortBylineText")
            .or_else(|| p.get("longBylineText"))
            .map(join_runs)
            .unwrap_or_else(|| "Unknown Artist".to_owned());
        let duration_ms = p
            .get("lengthText")
            .map(join_runs)
            .map(|t| parse_duration_text(&t))
            .unwrap_or(0);
        tracks.push(Track {
            id: video_id.to_owned(),
            title,
            artists: vec![ArtistRef {
                id: None,
                name: artist,
            }],
            album: None,
            duration_ms,
            artwork_url: best_thumbnail(p),
            is_explicit: false,
            is_liked: false,
            is_video: false,
        });
    }
    Ok(tracks)
}

/// Extracts lyrics browseId from a `/next` response for YTM internal lyrics.
pub fn extract_lyrics_browse_id(next_resp: &Value) -> Option<String> {
    let tabs = next_resp
        .pointer("/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs")
        .and_then(Value::as_array)?;
    for tab in tabs {
        if let Some(id) = tab
            .pointer("/tabRenderer/endpoint/browseEndpoint/browseId")
            .and_then(Value::as_str)
            && id.starts_with("MPLYt")
        {
            return Some(id.to_owned());
        }
    }
    None
}

/// Parses YTM internal plain lyrics from `/browse` (`MPLYt...`).
pub fn parse_ytm_lyrics_response(video_id: &str, resp: &Value) -> Option<LyricsDocument> {
    let text = resp
        .pointer("/contents/sectionListRenderer/contents/0/musicDescriptionShelfRenderer/description")
        .map(join_runs)?;
    let lines: Vec<LyricLine> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|line| LyricLine {
            start_ms: 0,
            end_ms: 0,
            text: line.to_owned(),
            words: Vec::new(),
            is_instrumental: false,
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(LyricsDocument {
        video_id: video_id.to_owned(),
        provider: "YouTube Music".to_owned(),
        sync_kind: LyricSyncKind::Plain,
        lines,
    })
}

/// Candidate format extracted from `/player` `streamingData.adaptiveFormats`.
#[derive(Debug, Clone)]
pub struct RawAudioFormat {
    pub itag: u32,
    pub url: Option<String>,
    pub signature_cipher: Option<String>,
    pub mime_type: String,
    pub codec: AudioCodec,
    pub bitrate_kbps: u32,
    pub content_length: u64,
    pub duration_ms: u64,
    pub loudness_db: Option<f32>,
}

/// Preferred audio itag order per RESEARCH.md Rule #4:
/// Opus 774 (~256 kbps Premium) -> Opus 251 (~128-160 kbps Free) -> AAC 141 -> AAC 140 -> HE-AAC 139.
pub const PREFERRED_AUDIO_ITAGS: &[u32] = &[774, 251, 141, 140, 139];

/// Parses `/player` response and picks the highest-priority audio format for Android/Premium default.
pub fn parse_player_response(
    video_id: &str,
    client_name: &str,
    resp: &Value,
    now_unix_secs: u64,
) -> CoreResult<RawAudioFormat> {
    parse_player_response_for_platform(
        video_id,
        client_name,
        resp,
        now_unix_secs,
        crate::innertube::formats::TargetPlatform::Android,
        crate::innertube::matrix::AccountTier::PremiumSignedIn,
    )
}

/// Parses `/player` response gated by native target decoder support (`TargetPlatform`)
/// and account tier (`AccountTier::Anonymous | FreeSignedIn | PremiumSignedIn`).
pub fn parse_player_response_for_platform(
    video_id: &str,
    client_name: &str,
    resp: &Value,
    now_unix_secs: u64,
    platform: crate::innertube::formats::TargetPlatform,
    tier: crate::innertube::matrix::AccountTier,
) -> CoreResult<RawAudioFormat> {
    let playability = &resp["playabilityStatus"];
    let status = playability["status"].as_str().unwrap_or("ERROR");
    if status != "OK" {
        return Err(CoreError::Provider(ProviderError::Unplayable {
            status: status.to_owned(),
            reason: playability["reason"]
                .as_str()
                .unwrap_or("video unavailable")
                .to_owned(),
        }));
    }

    let formats = resp
        .pointer("/streamingData/adaptiveFormats")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CoreError::Provider(ProviderError::NoStream {
                video_id: video_id.to_owned(),
            })
        })?;

    let track_loudness = resp
        .pointer("/playerConfig/audioConfig/loudnessDb")
        .and_then(Value::as_f64)
        .map(|v| v as f32);

    let _ = (client_name, now_unix_secs);
    let preferred_itags = platform.preferred_audio_itags(tier);

    for &itag in preferred_itags {
        if let Some(f) = formats
            .iter()
            .find(|f| f.get("itag").and_then(Value::as_u64) == Some(u64::from(itag)))
        {
            let mime_type = f
                .get("mimeType")
                .and_then(Value::as_str)
                .unwrap_or("audio/webm")
                .to_owned();
            let codec = if matches!(itag, 774 | 251) || mime_type.contains("opus") {
                AudioCodec::Opus
            } else {
                AudioCodec::Aac
            };
            let bitrate_kbps = f
                .get("averageBitrate")
                .or_else(|| f.get("bitrate"))
                .and_then(Value::as_u64)
                .unwrap_or(128_000) as u32
                / 1000;
            let content_length = f
                .get("contentLength")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let duration_ms = f
                .get("approxDurationMs")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let loudness_db = f
                .get("loudnessDb")
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .or(track_loudness);

            return Ok(RawAudioFormat {
                itag,
                url: f.get("url").and_then(Value::as_str).map(str::to_owned),
                signature_cipher: f
                    .get("signatureCipher")
                    .or_else(|| f.get("cipher"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                mime_type,
                codec,
                bitrate_kbps,
                content_length,
                duration_ms,
                loudness_db,
            });
        }
    }

    Err(CoreError::Provider(ProviderError::NoStream {
        video_id: video_id.to_owned(),
    }))
}

pub fn finalize_stream_info(
    video_id: &str,
    client_name: &str,
    raw: RawAudioFormat,
    resolved_url: String,
    now_unix_secs: u64,
) -> StreamInfo {
    // Cap cached stream TTL to <= 4h per RESEARCH.md Section 4.
    let expires_at_unix_secs = now_unix_secs + 4 * 3600;
    StreamInfo {
        video_id: video_id.to_owned(),
        url: resolved_url,
        itag: raw.itag,
        mime_type: raw.mime_type,
        codec: raw.codec,
        bitrate_kbps: raw.bitrate_kbps,
        content_length: raw.content_length,
        duration_ms: raw.duration_ms,
        loudness_db: raw.loudness_db,
        resolved_by_client: client_name.to_owned(),
        expires_at_unix_secs,
    }
}
