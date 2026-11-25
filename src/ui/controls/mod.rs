//! Styled reusable NSH UI controls organized in an atomic hierarchy:
//! - [`primitives`]: Buttons, chips, artwork tiles, switches
//! - [`media_rows`]: Compact mix rows, full track rows, loading skeletons
//! - [`cards`]: Square media shelf cards, status cards, diagnostics badges
//! - [`transport`]: Seek/volume sliders, Desktop Player Bar, Mobile Mini-Player
//! - [`sheets`]: Modal panel containers & action sheet rows
//! - [`navigation`]: Navigation pills & section headers
//!
//! All modules consume semantic tokens from `crate::ui::tokens` + `nsh_base::theme()`
//! with zero raw color literals.

pub mod cards;
pub mod media_rows;
pub mod navigation;
pub mod primitives;
pub mod sheets;
pub mod transport;

pub use cards::{diagnostics_badge, media_square_card, status_card};
pub use media_rows::{compact_mix_row, skeleton_row, track_row_view};
pub use navigation::{nav_pill_button, section_header_row};
pub use primitives::{
    artwork_box, glass_pill_button, hero_pill_button, icon_button, mood_chip, primary_button,
    secondary_button, setting_switch, svg_icon_button,
};
pub use sheets::{
    action_sheet_row, add_to_playlist_sheet, modal_panel_container, track_action_sheet,
};
pub use transport::{desktop_player_bar, mini_player_bar, seek_slider, volume_slider};
