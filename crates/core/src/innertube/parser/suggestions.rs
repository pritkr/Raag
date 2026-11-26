//! InnerTube `/youtubei/v1/music/get_search_suggestions` endpoint parser.

use resonance_contract::CoreResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::innertube::parser::common::{best_thumbnail, join_runs};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuggestionEntityKind {
    Song,
    Artist,
    Album,
    Playlist,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchSuggestionItem {
    QueryCompletion {
        text: String,
    },
    DirectEntity {
        kind: SuggestionEntityKind,
        title: String,
        subtitle: String,
        id: String,
        artwork_url: Option<String>,
    },
}

/// Parses `/music/get_search_suggestions` into structured [`SearchSuggestionItem`]s.
pub fn parse_search_suggestions_response(resp: &Value) -> CoreResult<Vec<SearchSuggestionItem>> {
    let sections = resp
        .get("contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    let mut out = Vec::new();
    for sec in sections {
        let items = sec
            .pointer("/searchSuggestionsSectionRenderer/contents")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();

        for item in items {
            if let Some(sugg) = item.get("searchSuggestionRenderer") {
                let text = sugg
                    .get("suggestion")
                    .map(join_runs)
                    .or_else(|| {
                        sugg.pointer("/navigationEndpoint/searchEndpoint/query")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .unwrap_or_default();
                if !text.is_empty() {
                    out.push(SearchSuggestionItem::QueryCompletion { text });
                }
            } else if let Some(entity) = item.get("musicResponsiveListItemRenderer") {
                let flex = entity
                    .get("flexColumns")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let title = flex
                    .first()
                    .and_then(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text"))
                    .map(join_runs)
                    .unwrap_or_default();
                let subtitle = flex
                    .get(1)
                    .and_then(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text"))
                    .map(join_runs)
                    .unwrap_or_default();
                if title.is_empty() {
                    continue;
                }
                if let Some(video_id) = entity
                    .pointer("/navigationEndpoint/watchEndpoint/videoId")
                    .or_else(|| {
                        entity.pointer(
                            "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId",
                        )
                    })
                    .and_then(Value::as_str)
                {
                    out.push(SearchSuggestionItem::DirectEntity {
                        kind: SuggestionEntityKind::Song,
                        title,
                        subtitle,
                        id: video_id.to_owned(),
                        artwork_url: best_thumbnail(entity),
                    });
                } else if let Some(browse_id) = entity
                    .pointer("/navigationEndpoint/browseEndpoint/browseId")
                    .and_then(Value::as_str)
                {
                    let kind = if browse_id.starts_with("UC") {
                        SuggestionEntityKind::Artist
                    } else if browse_id.starts_with("MPRE") || browse_id.starts_with("OLAK") {
                        SuggestionEntityKind::Album
                    } else {
                        SuggestionEntityKind::Playlist
                    };
                    out.push(SearchSuggestionItem::DirectEntity {
                        kind,
                        title,
                        subtitle,
                        id: browse_id.to_owned(),
                        artwork_url: best_thumbnail(entity),
                    });
                }
            }
        }
    }
    Ok(out)
}

/// Flattens [`SearchSuggestionItem`]s into plain query strings for simple autocomplete lists.
pub fn parse_search_suggestions_strings(resp: &Value) -> Vec<String> {
    parse_search_suggestions_response(resp)
        .unwrap_or_default()
        .into_iter()
        .map(|item| match item {
            SearchSuggestionItem::QueryCompletion { text } => text,
            SearchSuggestionItem::DirectEntity { title, .. } => title,
        })
        .collect()
}
