//! InnerTube `/youtubei/v1/browse` (`UC...` artist channel) parser.

use resonance_contract::{ArtistDetail, CoreResult, LibraryCollectionRef, Track};
use serde_json::Value;

use crate::innertube::parser::common::{best_thumbnail, join_runs, parse_music_responsive_item};

/// Parses an InnerTube artist `/browse` payload into [`ArtistDetail`].
pub fn parse_artist_response(channel_id: &str, resp: &Value) -> CoreResult<ArtistDetail> {
    let header = resp
        .pointer("/header/musicImmersiveHeaderRenderer")
        .or_else(|| resp.pointer("/header/musicVisualHeaderRenderer"))
        .or_else(|| resp.pointer("/header/musicDetailHeaderRenderer"));

    let name = header
        .and_then(|h| h.get("title"))
        .map(join_runs)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| channel_id.to_owned());

    let description = header
        .and_then(|h| h.get("description"))
        .map(join_runs)
        .filter(|s| !s.is_empty());

    let subscribers = header
        .and_then(|h| {
            h.pointer("/subscriptionButton/subscribeButtonRenderer/subscriberCountText")
                .or_else(|| {
                    h.pointer(
                        "/subscriptionButton/subscribeButtonRenderer/shortSubscriberCountText",
                    )
                })
        })
        .map(join_runs)
        .filter(|s| !s.is_empty());

    let is_subscribed = header
        .and_then(|h| h.pointer("/subscriptionButton/subscribeButtonRenderer/subscribed"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let artwork_url = header.and_then(best_thumbnail);

    let radio_playlist_id = header
        .and_then(|h| {
            h.pointer(
                "/startRadioButton/buttonRenderer/navigationEndpoint/watchEndpoint/playlistId",
            )
        })
        .and_then(Value::as_str)
        .map(str::to_owned);

    let shuffle_playlist_id = header
        .and_then(|h| {
            h.pointer("/playButton/buttonRenderer/navigationEndpoint/watchEndpoint/playlistId")
        })
        .and_then(Value::as_str)
        .map(str::to_owned);

    let mut top_songs: Vec<Track> = Vec::new();
    let mut albums: Vec<LibraryCollectionRef> = Vec::new();
    let mut singles: Vec<LibraryCollectionRef> = Vec::new();
    let mut related_artists: Vec<LibraryCollectionRef> = Vec::new();

    let sections = resp
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    for sec in sections {
        if let Some(shelf) = sec.get("musicShelfRenderer") {
            let items = shelf
                .get("contents")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            for item in items {
                if let Some(track) = parse_music_responsive_item(item) {
                    top_songs.push(track);
                }
            }
        } else if let Some(carousel) = sec.get("musicCarouselShelfRenderer") {
            let sec_title = carousel
                .pointer("/header/musicCarouselShelfBasicHeaderRenderer/title")
                .map(join_runs)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let items = carousel
                .get("contents")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            for item in items {
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
                let entry = LibraryCollectionRef {
                    browse_id,
                    playlist_id: None,
                    title,
                    subtitle: two_row.get("subtitle").map(join_runs).unwrap_or_default(),
                    artwork_url: best_thumbnail(two_row),
                    track_count: None,
                };
                if sec_title.contains("single") {
                    singles.push(entry);
                } else if sec_title.contains("fans") || sec_title.contains("similar") {
                    related_artists.push(entry);
                } else {
                    albums.push(entry);
                }
            }
        }
    }

    Ok(ArtistDetail {
        channel_id: channel_id.to_owned(),
        name,
        description,
        subscribers,
        is_subscribed,
        artwork_url,
        radio_playlist_id,
        shuffle_playlist_id,
        top_songs,
        albums,
        singles,
        related_artists,
    })
}
