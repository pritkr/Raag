//! InnerTube `/youtubei/v1/search` endpoint parser.

use resonance_contract::{CoreResult, Playlist, SearchResults};
use serde_json::Value;

use crate::innertube::parser::common::{best_thumbnail, join_runs, parse_music_responsive_item};

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
