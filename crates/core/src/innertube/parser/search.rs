//! InnerTube `/youtubei/v1/search` endpoint parser.

use std::collections::HashMap;

use resonance_contract::{ArtistRef, CoreResult, Playlist, SearchResults, Track};
use serde_json::Value;

use crate::innertube::parser::common::{best_thumbnail, join_runs, parse_music_responsive_item};

fn parse_responsive_collection_item(item: &Value) -> Option<Playlist> {
    let r = item.get("musicResponsiveListItemRenderer").unwrap_or(item);
    let browse_id = r
        .pointer("/navigationEndpoint/browseEndpoint/browseId")
        .or_else(|| {
            r.pointer(
                "/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text/runs/0/navigationEndpoint/browseEndpoint/browseId",
            )
        })
        .and_then(Value::as_str)?;
    let title = r
        .pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text")
        .map(join_runs)
        .filter(|s| !s.is_empty())?;
    let subtitle = r
        .pointer("/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text")
        .map(join_runs)
        .filter(|s| !s.is_empty());
    Some(Playlist {
        id: browse_id.to_owned(),
        title,
        subtitle,
        artwork_url: best_thumbnail(r),
        tracks: Vec::new(),
    })
}

fn push_responsive_search_item(item: &Value, tracks: &mut Vec<Track>, albums: &mut Vec<Playlist>) {
    if let Some(track) = parse_music_responsive_item(item) {
        tracks.push(track);
    } else if let Some(col) = parse_responsive_collection_item(item) {
        albums.push(col);
    }
}

/// Extracts `video_id -> plays/views text` (e.g. `"1.2B plays"`) from a `WEB_REMIX` search response.
pub fn extract_search_plays(resp: &Value) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let tabs = resp
        .pointer("/contents/tabbedSearchResultsRenderer/tabs")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    let mut collect_item = |item: &Value| {
        let r = item.get("musicResponsiveListItemRenderer").unwrap_or(item);
        let Some(vid) = r
            .pointer("/playlistItemData/videoId")
            .or_else(|| {
                r.pointer("/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId")
            })
            .and_then(Value::as_str)
        else {
            return;
        };
        if let Some(flex) = r.get("flexColumns").and_then(Value::as_array) {
            for col in flex.iter().skip(1) {
                let txt = col
                    .pointer("/musicResponsiveListItemFlexColumnRenderer/text")
                    .map(join_runs)
                    .unwrap_or_default();
                for part in txt.split(['•', '·']) {
                    let p = part.trim();
                    if p.ends_with(" plays") || p.ends_with(" views") {
                        out.insert(vid.to_owned(), p.to_owned());
                        return;
                    }
                }
            }
        }
    };

    for tab in tabs {
        let sections = tab
            .pointer("/tabRenderer/content/sectionListRenderer/contents")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for section in sections {
            for key in [
                "musicShelfRenderer",
                "itemSectionRenderer",
                "musicCardShelfRenderer",
            ] {
                if let Some(items) = section
                    .pointer(&format!("/{key}/contents"))
                    .and_then(Value::as_array)
                {
                    for item in items {
                        collect_item(item);
                    }
                }
            }
        }
    }
    out
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
                    push_responsive_search_item(item, &mut tracks, &mut albums);
                }
            } else if let Some(item_sec) = section.get("itemSectionRenderer") {
                let items = item_sec
                    .get("contents")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                for item in items {
                    push_responsive_search_item(item, &mut tracks, &mut albums);
                }
            } else if let Some(card) = section.get("musicCardShelfRenderer") {
                let card_title = card.get("title").map(join_runs).unwrap_or_default();
                let card_sub = card.get("subtitle").map(join_runs).unwrap_or_default();
                let card_art = best_thumbnail(card);
                if let Some(video_id) = card
                    .pointer("/onTap/watchEndpoint/videoId")
                    .or_else(|| {
                        card.pointer("/title/runs/0/navigationEndpoint/watchEndpoint/videoId")
                    })
                    .and_then(Value::as_str)
                    && !card_title.is_empty()
                {
                    tracks.push(Track {
                        id: video_id.to_owned(),
                        title: card_title,
                        artists: if card_sub.is_empty() {
                            Vec::new()
                        } else {
                            vec![ArtistRef {
                                id: None,
                                name: card_sub,
                            }]
                        },
                        album: None,
                        duration_ms: 0,
                        artwork_url: card_art,
                        is_explicit: false,
                        is_liked: false,
                        is_video: false,
                    });
                } else if let Some(browse_id) = card
                    .pointer("/onTap/browseEndpoint/browseId")
                    .or_else(|| {
                        card.pointer("/title/runs/0/navigationEndpoint/browseEndpoint/browseId")
                    })
                    .and_then(Value::as_str)
                    && !card_title.is_empty()
                {
                    albums.push(Playlist {
                        id: browse_id.to_owned(),
                        title: card_title,
                        subtitle: if card_sub.is_empty() {
                            None
                        } else {
                            Some(card_sub)
                        },
                        artwork_url: card_art,
                        tracks: Vec::new(),
                    });
                }

                if let Some(items) = card.get("contents").and_then(Value::as_array) {
                    for item in items {
                        push_responsive_search_item(item, &mut tracks, &mut albums);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_search_tracks_albums_and_continuation() {
        let payload = json!({
            "contents": {
                "tabbedSearchResultsRenderer": {
                    "tabs": [{
                        "tabRenderer": {
                            "content": {
                                "sectionListRenderer": {
                                    "contents": [{
                                        "musicShelfRenderer": {
                                            "continuations": [{"nextContinuationData": {"continuation": "tok_next"}}],
                                            "contents": [
                                                {
                                                    "musicResponsiveListItemRenderer": {
                                                        "playlistItemData": {"videoId": "vid_srch_1"},
                                                        "flexColumns": [
                                                            {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Aruarian Dance"}]}}},
                                                            {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Nujabes"}, {"text": " • "}, {"text": "4:10"}]}}}
                                                        ]
                                                    }
                                                },
                                                {
                                                    "musicResponsiveListItemRenderer": {
                                                        "navigationEndpoint": {"browseEndpoint": {"browseId": "MPREb_123"}},
                                                        "flexColumns": [
                                                            {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Modal Soul"}]}}},
                                                            {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Album • 2005"}]}}}
                                                        ]
                                                    }
                                                }
                                            ]
                                        }
                                    }]
                                }
                            }
                        }
                    }]
                }
            }
        });
        let res = parse_search_response("nujabes", &payload).unwrap();
        assert_eq!(res.tracks.len(), 1);
        assert_eq!(res.tracks[0].duration_ms, 250_000);
        assert_eq!(res.albums.len(), 1);
        assert_eq!(res.continuation.as_deref(), Some("tok_next"));
    }
}
