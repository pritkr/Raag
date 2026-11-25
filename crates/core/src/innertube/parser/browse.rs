//! InnerTube `/youtubei/v1/browse` endpoint parser for albums, playlists, liked songs, and YTM lyrics.

use resonance_contract::{CoreResult, LyricLine, LyricSyncKind, LyricsDocument, Playlist};
use serde_json::Value;

use crate::innertube::parser::common::{best_thumbnail, join_runs, parse_music_responsive_item};

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_two_column_collection_and_plain_lyrics() {
        let collection = json!({
            "header": {
                "musicResponsiveHeaderRenderer": {
                    "title": {"runs": [{"text": "Ambient Study"}]},
                    "subtitle": {"runs": [{"text": "Playlist • 2026"}]}
                }
            },
            "contents": {
                "twoColumnBrowseResultsRenderer": {
                    "secondaryContents": {
                        "sectionListRenderer": {
                            "contents": [{
                                "musicPlaylistShelfRenderer": {
                                    "contents": [{
                                        "musicResponsiveListItemRenderer": {
                                            "playlistItemData": {"videoId": "vid_col_1"},
                                            "flexColumns": [
                                                {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Drift"}]}}},
                                                {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Artist"}, {"text": " • "}, {"text": "2:30"}]}}}
                                            ]
                                        }
                                    }]
                                }
                            }]
                        }
                    }
                }
            }
        });
        let pl = parse_collection_response("VLPL_STUDY", &collection).unwrap();
        assert_eq!(pl.title, "Ambient Study");
        assert_eq!(pl.tracks.len(), 1);
        assert_eq!(pl.tracks[0].id, "vid_col_1");

        let lyrics_resp = json!({
            "contents": {
                "sectionListRenderer": {
                    "contents": [{
                        "musicDescriptionShelfRenderer": {
                            "description": {"runs": [{"text": "Line one\n\nLine two"}]}
                        }
                    }]
                }
            }
        });
        let doc = parse_ytm_lyrics_response("vid_col_1", &lyrics_resp).unwrap();
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.sync_kind, LyricSyncKind::Plain);
    }
}
