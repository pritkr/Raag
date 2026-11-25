//! Shared InnerTube JSON extraction helpers used across endpoint parsers.

use resonance_contract::{AlbumRef, ArtistRef, Track};
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

pub(crate) fn join_runs(v: &Value) -> String {
    v.get("runs")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(|r| r.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .or_else(|| {
            v.get("simpleText")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

pub(crate) fn best_thumbnail(v: &Value) -> Option<String> {
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

pub(crate) fn parse_music_responsive_item(item: &Value) -> Option<Track> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_durations_and_visitor_data() {
        assert_eq!(parse_duration_text("3:45"), 225_000);
        assert_eq!(parse_duration_text("1:02:10"), 3_730_000);
        assert_eq!(parse_duration_text("42"), 42_000);
        assert_eq!(parse_duration_text("invalid"), 0);

        let resp = json!({"responseContext": {"visitorData": "Cgt2aXNpdG9y"}});
        assert_eq!(extract_visitor_data(&resp).as_deref(), Some("Cgt2aXNpdG9y"));
        assert_eq!(extract_visitor_data(&json!({})), None);
    }

    #[test]
    fn picks_highest_width_thumbnail() {
        let v = json!({
            "thumbnail": {
                "thumbnails": [
                    {"url": "https://img/small.jpg", "width": 120},
                    {"url": "https://img/large.jpg", "width": 544}
                ]
            }
        });
        assert_eq!(best_thumbnail(&v).as_deref(), Some("https://img/large.jpg"));
    }
}
