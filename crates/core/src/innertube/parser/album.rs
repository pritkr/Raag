//! InnerTube `/youtubei/v1/browse` (`MPRE...` / `OLAK...` album) parser.

use resonance_contract::{AlbumDetail, ArtistRef, CoreResult, LibraryCollectionRef, Track};
use serde_json::Value;

use crate::innertube::parser::browse::parse_collection_response;
use crate::innertube::parser::common::{best_thumbnail, join_runs};

/// Parses an InnerTube album `/browse` payload into [`AlbumDetail`].
pub fn parse_album_response(browse_id: &str, resp: &Value) -> CoreResult<AlbumDetail> {
    let playlist = parse_collection_response(browse_id, resp)?;

    let header = resp
        .pointer("/header/musicDetailHeaderRenderer")
        .or_else(|| resp.pointer("/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer"));

    let mut artists: Vec<ArtistRef> = Vec::new();
    let mut release_year = None;
    let mut release_type = "Album".to_owned();

    if let Some(runs) = header
        .and_then(|h| h.pointer("/subtitle/runs"))
        .and_then(Value::as_array)
    {
        for run in runs {
            let text = run.get("text").and_then(Value::as_str).unwrap_or("").trim();
            if text.is_empty() || text == "•" || text == "·" || text == "," {
                continue;
            }
            if let Some(artist_id) = run
                .pointer("/navigationEndpoint/browseEndpoint/browseId")
                .and_then(Value::as_str)
            {
                artists.push(ArtistRef {
                    id: Some(artist_id.to_owned()),
                    name: text.to_owned(),
                });
            } else if text.len() == 4 && text.chars().all(|c| c.is_ascii_digit()) {
                release_year = Some(text.to_owned());
            } else if matches!(
                text.to_ascii_lowercase().as_str(),
                "album" | "ep" | "single"
            ) {
                release_type = text.to_owned();
            }
        }
    }

    let audio_playlist_id = header
        .and_then(|h| {
            h.pointer("/menu/menuRenderer/topLevelButtons/0/buttonRenderer/navigationEndpoint/watchPlaylistEndpoint/playlistId")
        })
        .and_then(Value::as_str)
        .map(str::to_owned);

    let description = header
        .and_then(|h| h.get("description"))
        .map(join_runs)
        .filter(|s| !s.is_empty())
        .or_else(|| playlist.subtitle.clone());

    let artwork_url = header
        .and_then(best_thumbnail)
        .or_else(|| playlist.artwork_url.clone());

    let total_duration_ms: u64 = playlist.tracks.iter().map(|t| t.duration_ms).sum();
    let tracks: Vec<Track> = playlist.tracks;

    Ok(AlbumDetail {
        browse_id: browse_id.to_owned(),
        audio_playlist_id,
        title: playlist.title,
        artists,
        release_year,
        release_type,
        description,
        artwork_url,
        total_duration_ms,
        tracks,
        other_versions: Vec::<LibraryCollectionRef>::new(),
    })
}
