//! InnerTube `/youtubei/v1/browse` parsers for `FEmusic_explore`, `FEmusic_charts`,
//! `FEmusic_moods_and_genres`, mood playlist drill-downs, and Home mood chips.

use resonance_contract::{
    CoreResult, ExplorePage, HomeSection, LibraryCollectionRef, MoodCategory, Playlist,
};
use serde_json::Value;

use crate::innertube::parser::common::{best_thumbnail, join_runs};
use crate::innertube::parser::home::parse_home_response;

/// Extracts dynamic mood filter chips (`chipCloudRenderer.chips[*]`) from `FEmusic_home`.
pub fn extract_home_mood_chips(resp: &Value) -> Vec<MoodCategory> {
    let chips = resp
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/header/chipCloudRenderer/chips")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    let mut out = Vec::new();
    for chip in chips {
        let Some(r) = chip.get("chipCloudChipRenderer") else {
            continue;
        };
        let title = r.get("text").map(join_runs).unwrap_or_default();
        let params = r
            .pointer("/navigationEndpoint/browseEndpoint/params")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if !title.is_empty() && !params.is_empty() {
            out.push(MoodCategory {
                title,
                params,
                accent_hint: None,
            });
        }
    }
    out
}

/// Parses `FEmusic_explore` (or `FEmusic_moods_and_genres`) into [`ExplorePage`].
pub fn parse_explore_response(resp: &Value) -> CoreResult<ExplorePage> {
    let sections = parse_home_response(resp)?;
    let mut new_releases = Vec::new();
    let mut charts: Vec<HomeSection> = Vec::new();
    let mut moods_and_genres = Vec::new();

    for sec in &sections {
        if sec.title.to_ascii_lowercase().contains("new")
            || sec.title.to_ascii_lowercase().contains("release")
        {
            for item in &sec.items {
                if let resonance_contract::HomeItem::Collection {
                    browse_id,
                    title,
                    subtitle,
                    artwork_url,
                } = item
                {
                    new_releases.push(Playlist {
                        id: browse_id.clone(),
                        title: title.clone(),
                        subtitle: if subtitle.is_empty() {
                            None
                        } else {
                            Some(subtitle.clone())
                        },
                        artwork_url: artwork_url.clone(),
                        tracks: Vec::new(),
                    });
                }
            }
        } else {
            charts.push(sec.clone());
        }
    }

    let contents = resp
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .or_else(|| resp.pointer("/continuationContents/sectionListContinuation/contents"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    for sec in contents {
        let Some(grid) = sec.get("gridRenderer") else {
            continue;
        };
        let Some(items) = grid.get("items").and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let Some(btn) = item.get("musicNavigationButtonRenderer") else {
                continue;
            };
            let title = btn.get("buttonText").map(join_runs).unwrap_or_default();
            let params = btn
                .pointer("/clickCommand/browseEndpoint/params")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if !title.is_empty() && !params.is_empty() {
                let accent_hint = btn
                    .pointer("/solid/leftStripeColor")
                    .and_then(Value::as_u64)
                    .map(|c| format!("#{:08X}", c));
                moods_and_genres.push(MoodCategory {
                    title,
                    params,
                    accent_hint,
                });
            }
        }
    }

    Ok(ExplorePage {
        new_releases,
        charts,
        moods_and_genres,
    })
}

/// Parses `FEmusic_moods_and_genres_category` (mood drill-down) into [`LibraryCollectionRef`]s.
pub fn parse_mood_playlists_response(resp: &Value) -> CoreResult<Vec<LibraryCollectionRef>> {
    let contents = resp
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    let mut out = Vec::new();
    for sec in contents {
        let raw_items = sec
            .pointer("/musicCarouselShelfRenderer/contents")
            .or_else(|| sec.pointer("/gridRenderer/items"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();

        for item in raw_items {
            let Some(two_row) = item.get("musicTwoRowItemRenderer") else {
                continue;
            };
            let title = two_row.get("title").map(join_runs).unwrap_or_default();
            let browse_id = two_row
                .pointer("/navigationEndpoint/browseEndpoint/browseId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if title.is_empty() || browse_id.is_empty() {
                continue;
            }
            let subtitle = two_row.get("subtitle").map(join_runs).unwrap_or_default();
            let playlist_id = browse_id.strip_prefix("VL").map(str::to_owned);
            out.push(LibraryCollectionRef {
                browse_id,
                playlist_id,
                title,
                subtitle,
                artwork_url: best_thumbnail(two_row),
                track_count: None,
            });
        }
    }
    Ok(out)
}
