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
pub mod cast_sheet;
pub mod context_menu;
pub mod media_rows;
pub mod navigation;
pub mod primitives;
pub mod sheets;
pub mod sleep_timer_sheet;
pub mod speed_sheet;
pub mod transport;
pub mod vault_widgets;

pub use cards::{diagnostics_badge, media_square_card, status_card};
pub use cast_sheet::{active_cast_banner, cast_button, cast_picker_sheet};
pub use context_menu::{TrackMenuContext, track_context_menu};
pub use media_rows::{compact_mix_row, skeleton_row, track_row_view};
pub use navigation::{
    desktop_responsive_shell_helpers, dispatch_global_shortcut, is_text_input_focused,
    mobile_bottom_nav_bar, nav_pill_button, register_global_shortcuts, section_header_row,
    set_text_input_focused,
};
pub use primitives::{
    artwork_box, glass_pill_button, hero_pill_button, icon_button, mood_chip, primary_button,
    secondary_button, setting_switch, svg_icon_button,
};
pub use sheets::{
    action_sheet_row, add_to_playlist_sheet, modal_panel_container, track_action_sheet,
};
pub use sleep_timer_sheet::{open_sleep_timer_sheet, sleep_timer_badge_label, sleep_timer_sheet};
pub use speed_sheet::{current_speed_config, open_speed_sheet, speed_badge_label, speed_sheet};
pub use transport::{desktop_player_bar, mini_player_bar, seek_slider, volume_slider};
pub use vault_widgets::{
    OfflineStorageSettingsProps, collection_pin_button, offline_storage_settings_card,
};
