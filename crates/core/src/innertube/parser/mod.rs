//! Pure, deterministic InnerTube JSON parsers decomposed by endpoint:
//! - [`common`] — visitor data, duration text, thumbnails, responsive item extraction
//! - [`home`] — `/browse` (`FEmusic_home`) carousels
//! - [`search`] — `/search` shelves & continuations
//! - [`browse`] — `/browse` albums, playlists, liked songs, and YTM lyrics
//! - [`player`] — `/player` adaptive audio format selection & stream finalization
//! - [`next`] — `/next` autoplay radio queue & lyrics tab browseId
//!
//! Endpoint schema changes from YouTube stay strictly isolated to their own module.

pub mod album;
pub mod artist;
pub mod browse;
pub mod common;
pub mod explore;
pub mod home;
pub mod next;
pub mod player;
pub mod search;
pub mod suggestions;

pub use album::parse_album_response;
pub use artist::parse_artist_response;
pub use browse::{parse_collection_response, parse_ytm_lyrics_response};
pub use common::{extract_visitor_data, parse_duration_text};
pub use explore::{extract_home_mood_chips, parse_explore_response, parse_mood_playlists_response};
pub use home::{extract_home_continuation, parse_home_response};
pub use next::{extract_lyrics_browse_id, parse_next_queue_response};
pub use player::{
    PREFERRED_AUDIO_ITAGS, RawAudioFormat, finalize_stream_info, parse_player_response,
    parse_player_response_for_platform,
};
pub use search::{extract_search_plays, parse_search_response};
pub use suggestions::{
    SearchSuggestionItem, SuggestionEntityKind, parse_search_suggestions_response,
    parse_search_suggestions_strings,
};
