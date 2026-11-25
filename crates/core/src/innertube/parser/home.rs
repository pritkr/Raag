//! InnerTube `/youtubei/v1/browse` (`FEmusic_home`) endpoint parser.

use resonance_contract::{ArtistRef, CoreResult, HomeItem, HomeSection, Track};
use serde_json::Value;

use crate::innertube::parser::common::{best_thumbnail, join_runs, parse_music_responsive_item};

/// Extracts `nextContinuationData.continuation` from a `/browse` (`FEmusic_home`) response.
pub fn extract_home_continuation(resp: &Value) -> Option<String> {
    resp.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/continuations/0/nextContinuationData/continuation")
        .or_else(|| resp.pointer("/continuationContents/sectionListContinuation/continuations/0/nextContinuationData/continuation"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn parse_home_item(raw: &Value) -> Option<HomeItem> {
    if let Some(track) = parse_music_responsive_item(raw) {
        return Some(HomeItem::Track(track));
    }
    let two_row = raw.get("musicTwoRowItemRenderer")?;
    let item_title = two_row.get("title").map(join_runs).unwrap_or_default();
    if item_title.is_empty() {
        return None;
    }
    let subtitle = two_row.get("subtitle").map(join_runs).unwrap_or_default();
    let artwork_url = best_thumbnail(two_row);
    if let Some(video_id) = two_row
        .pointer("/navigationEndpoint/watchEndpoint/videoId")
        .and_then(Value::as_str)
    {
        return Some(HomeItem::Track(Track {
            id: video_id.to_owned(),
            title: item_title,
            artists: if subtitle.is_empty() {
                Vec::new()
            } else {
                vec![ArtistRef {
                    id: None,
                    name: subtitle,
                }]
            },
            album: None,
            duration_ms: 0,
            artwork_url,
            is_explicit: false,
            is_liked: false,
            is_video: false,
        }));
    }
    if let Some(browse_id) = two_row
        .pointer("/navigationEndpoint/browseEndpoint/browseId")
        .and_then(Value::as_str)
    {
        return Some(HomeItem::Collection {
            browse_id: browse_id.to_owned(),
            title: item_title,
            subtitle,
            artwork_url,
        });
    }
    if let Some(video_id) = two_row
        .pointer("/thumbnailOverlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId")
        .and_then(Value::as_str)
    {
        return Some(HomeItem::Track(Track {
            id: video_id.to_owned(),
            title: item_title,
            artists: if subtitle.is_empty() {
                Vec::new()
            } else {
                vec![ArtistRef {
                    id: None,
                    name: subtitle,
                }]
            },
            album: None,
            duration_ms: 0,
            artwork_url,
            is_explicit: false,
            is_liked: false,
            is_video: false,
        }));
    }
    None
}

/// Parses `WEB_REMIX` `/youtubei/v1/browse` (`FEmusic_home` / `FEmusic_explore` / continuations) into `HomeSection`s.
pub fn parse_home_response(resp: &Value) -> CoreResult<Vec<HomeSection>> {
    let mut sections_out = Vec::new();
    let contents = resp
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .or_else(|| resp.pointer("/continuationContents/sectionListContinuation/contents"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    for (idx, sec) in contents.iter().enumerate() {
        let (title, raw_items) = if let Some(carousel) = sec.get("musicCarouselShelfRenderer") {
            let title = carousel
                .pointer("/header/musicCarouselShelfBasicHeaderRenderer/title")
                .map(join_runs)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("Section {}", idx + 1));
            let raw_items = carousel
                .get("contents")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            (title, raw_items)
        } else if let Some(grid) = sec
            .get("musicGridRenderer")
            .or_else(|| sec.get("gridRenderer"))
        {
            let title = grid
                .pointer("/header/gridHeaderRenderer/title")
                .map(join_runs)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("Section {}", idx + 1));
            let raw_items = grid
                .get("items")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            (title, raw_items)
        } else {
            continue;
        };

        let items: Vec<HomeItem> = raw_items.iter().filter_map(parse_home_item).collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_home_carousels_with_tracks_and_collections() {
        let payload = json!({
            "contents": {
                "singleColumnBrowseResultsRenderer": {
                    "tabs": [{
                        "tabRenderer": {
                            "content": {
                                "sectionListRenderer": {
                                    "contents": [{
                                        "musicCarouselShelfRenderer": {
                                            "header": {
                                                "musicCarouselShelfBasicHeaderRenderer": {
                                                    "title": {"runs": [{"text": "Quick Picks"}]}
                                                }
                                            },
                                            "contents": [
                                                {
                                                    "musicTwoRowItemRenderer": {
                                                        "title": {"runs": [{"text": "Late Night Mix"}]},
                                                        "subtitle": {"runs": [{"text": "Playlist"}]},
                                                        "navigationEndpoint": {"browseEndpoint": {"browseId": "VLPL123"}}
                                                    }
                                                },
                                                {
                                                    "musicTwoRowItemRenderer": {
                                                        "title": {"runs": [{"text": "Midnight Drift"}]},
                                                        "subtitle": {"runs": [{"text": "Nujabes"}]},
                                                        "navigationEndpoint": {"watchEndpoint": {"videoId": "vid_home_1"}}
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
        let sections = parse_home_response(&payload).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].title, "Quick Picks");
        assert_eq!(sections[0].items.len(), 2);
    }
}
