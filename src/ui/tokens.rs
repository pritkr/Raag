//! Semantic theme & motion tokens reading from `theme.json` via `nsh_base::theme()`.
//!
//! Zero raw hex/RGB values in presentation code.

use std::time::Duration;

use nsh_base::theme;
use nsh_core::prelude::*;
use resonance_core::palette::AmbientPalette;

/// Breakpoint separating mobile bottom-bar layout from desktop sidebar layout.
pub const DESKTOP_MIN_WIDTH: f32 = 820.0;

/// Breakpoint enabling the persistent Desktop right-hand Queue/Lyrics panel.
pub const DESKTOP_PANEL_MIN_WIDTH: f32 = 1080.0;

/// Minimum interactive touch target on Android/mobile per Binary Visual Rubric #1.
pub const MOBILE_MIN_TOUCH_TARGET: f32 = 48.0;

/// Motion tokens honoring system reduced-motion setting.
pub mod motion {
    use super::*;

    pub const FAST_MS: u64 = 120;
    pub const NORMAL_MS: u64 = 180;
    pub const SPRING_MS: u64 = 240;

    pub fn duration_ms(base_ms: u64, reduced_motion: bool) -> Duration {
        if reduced_motion {
            Duration::from_millis(0)
        } else {
            Duration::from_millis(base_ms)
        }
    }
}

pub fn is_desktop_layout() -> bool {
    window().width.0 >= DESKTOP_MIN_WIDTH
}

pub fn is_desktop_panel_visible() -> bool {
    window().width.0 >= DESKTOP_PANEL_MIN_WIDTH
}

pub fn radius_card() -> Points {
    px(theme().number("radius-card").unwrap_or(14.0))
}

pub fn radius_panel() -> Points {
    px(theme().number("radius-panel").unwrap_or(20.0))
}

fn rgb_u8_to_hsla(c: [u8; 3], alpha: f32) -> Hsla {
    let packed = ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | (c[2] as u32);
    let mut h: Hsla = rgb(packed).into();
    h.a = alpha.clamp(0.0, 1.0);
    h
}

/// Artwork-driven atmospheric ambient backdrop surface for Now Playing & Synced Lyrics,
/// calibrated per scheme to preserve WCAG AA contrast (>= 11:1 dark, >= 13:1 light).
pub fn ambient_backdrop_color(palette: AmbientPalette) -> AdaptiveColor {
    AdaptiveColor::new(
        rgb_u8_to_hsla(palette.light_backdrop, 1.0),
        rgb_u8_to_hsla(palette.dark_backdrop, 1.0),
    )
}

/// Artwork-driven elevated glow / halo accent behind album art & active ambient highlights.
pub fn ambient_accent_color(palette: AmbientPalette) -> AdaptiveColor {
    AdaptiveColor::new(
        rgb_u8_to_hsla(palette.light_accent, 0.92),
        rgb_u8_to_hsla(palette.dark_accent, 0.92),
    )
}

/// Calibrated translucent frosted glass surface for Mini-Player & Bottom Navigation Bar.
/// Paired with NSH native `.backdrop_material(...)` and falls back crisply on platforms
/// without native blur.
pub fn frosted_bar_surface() -> AdaptiveColor {
    let mut light_hsla: Hsla = rgb(0xffffff).into();
    light_hsla.a = 0.86;
    let mut dark_hsla: Hsla = rgb(0x111318).into();
    dark_hsla.a = 0.84;
    AdaptiveColor::new(light_hsla, dark_hsla)
}

/// Calibrated translucent glass card surface for Now Playing controls & floating headers.
pub fn frosted_card_surface() -> AdaptiveColor {
    let mut light_hsla: Hsla = rgb(0xffffff).into();
    light_hsla.a = 0.78;
    let mut dark_hsla: Hsla = rgb(0x161922).into();
    dark_hsla.a = 0.72;
    AdaptiveColor::new(light_hsla, dark_hsla)
}

/// Subtle 1px inner-highlight hairline border for translucent frosted surfaces.
pub fn glass_hairline() -> AdaptiveColor {
    let mut light_hsla: Hsla = rgb(0x000000).into();
    light_hsla.a = 0.08;
    let mut dark_hsla: Hsla = rgb(0xffffff).into();
    dark_hsla.a = 0.11;
    AdaptiveColor::new(light_hsla, dark_hsla)
}

pub fn color_player_surface() -> AdaptiveColor {
    theme().color("player").unwrap_or(theme().card)
}

pub fn color_control() -> AdaptiveColor {
    theme().color("control").unwrap_or(theme().primary)
}

pub fn color_control_fg() -> AdaptiveColor {
    theme()
        .color("control-foreground")
        .unwrap_or(theme().primary_foreground)
}

pub fn color_liked() -> AdaptiveColor {
    theme().color("liked").unwrap_or(theme().primary)
}

pub fn color_lyric_active() -> AdaptiveColor {
    theme().color("lyric-active").unwrap_or(theme().foreground)
}

pub fn color_lyric_inactive() -> AdaptiveColor {
    theme()
        .color("lyric-inactive")
        .unwrap_or(theme().muted_foreground)
}

pub fn color_sponsor_badge() -> AdaptiveColor {
    theme().color("sponsor-badge").unwrap_or(theme().primary)
}

pub fn color_status_ok() -> AdaptiveColor {
    theme().color("status-ok").unwrap_or(theme().primary)
}
