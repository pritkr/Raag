//! InnerTube `/youtubei/v1/next` endpoint parser for autoplay radio queues and lyrics tabs.

use resonance_contract::{ArtistRef, CoreResult, Track};
use serde_json::Value;

use crate::innertube::parser::common::{best_thumbnail, join_runs, parse_duration_text};

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_next_radio_queue_and_lyrics_tab_id() {
        let resp = json!({
            "contents": {
                "singleColumnMusicWatchNextResultsRenderer": {
                    "tabbedRenderer": {
                        "watchNextTabbedResultsRenderer": {
                            "tabs": [
                                {
                                    "tabRenderer": {
                                        "content": {
                                            "musicQueueRenderer": {
                                                "content": {
                                                    "playlistPanelRenderer": {
                                                        "contents": [{
                                                            "playlistPanelVideoRenderer": {
                                                                "videoId": "vid_radio_1",
                                                                "title": {"runs": [{"text": "Feather"}]},
                                                                "shortBylineText": {"runs": [{"text": "Nujabes"}]},
                                                                "lengthText": {"runs": [{"text": "2:55"}]}
                                                            }
                                                        }]
                                                    }
                                                }
                                            }
                                        }
                                    }
                                },
                                {
                                    "tabRenderer": {
                                        "endpoint": {
                                            "browseEndpoint": {"browseId": "MPLYt_abc123"}
                                        }
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        });
        let queue = parse_next_queue_response(&resp).unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].duration_ms, 175_000);
        assert_eq!(
            extract_lyrics_browse_id(&resp).as_deref(),
            Some("MPLYt_abc123")
        );
    }
}
