//! Semantic theme & motion tokens reading from `theme.json` via `nsh_base::theme()`.
//!
//! Zero raw hex/RGB values in presentation code (`src/screens/`, `src/ui/controls.rs`).

use std::time::Duration;

use nsh_base::theme;
use nsh_core::prelude::*;
use resonance_core::palette::AmbientPalette;

/// Breakpoint separating mobile bottom-bar layout from desktop sidebar layout.
pub const DESKTOP_MIN_WIDTH: f32 = 820.0;

/// Breakpoint enabling the persistent Desktop right-hand Queue/Lyrics panel (when toggled or wide).
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
    px(theme().number("radius-card").unwrap_or(12.0))
}

pub fn radius_panel() -> Points {
    px(theme().number("radius-panel").unwrap_or(18.0))
}

pub fn radius_pill() -> Points {
    px(theme().number("radius-pill").unwrap_or(999.0))
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
        rgb_u8_to_hsla(palette.light_accent, 0.94),
        rgb_u8_to_hsla(palette.dark_accent, 0.94),
    )
}

/// Subtle warm top-ambient tint matching the reference YouTube Music Desktop canvas glow.
pub fn color_ambient_warm() -> AdaptiveColor {
    theme().color("ambient-warm").unwrap_or(theme().accent)
}

/// Calibrated translucent frosted glass surface for Mini-Player & Bottom Navigation Bar.
pub fn frosted_bar_surface() -> AdaptiveColor {
    let mut light_hsla: Hsla = rgb(0xffffff).into();
    light_hsla.a = 0.90;
    let mut dark_hsla: Hsla = rgb(0x0c0b0a).into();
    dark_hsla.a = 0.92;
    AdaptiveColor::new(light_hsla, dark_hsla)
}

/// Calibrated translucent glass card surface for Now Playing controls & floating headers.
pub fn frosted_card_surface() -> AdaptiveColor {
    let mut light_hsla: Hsla = rgb(0xffffff).into();
    light_hsla.a = 0.78;
    let mut dark_hsla: Hsla = rgb(0x181615).into();
    dark_hsla.a = 0.76;
    AdaptiveColor::new(light_hsla, dark_hsla)
}

/// Subtle translucent pill surface for search bar, mood chips, and action pills.
pub fn pill_glass_surface() -> AdaptiveColor {
    let mut light_hsla: Hsla = rgb(0x111214).into();
    light_hsla.a = 0.06;
    let mut dark_hsla: Hsla = rgb(0xffffff).into();
    dark_hsla.a = 0.08;
    AdaptiveColor::new(light_hsla, dark_hsla)
}

/// Hover state for translucent pills.
pub fn pill_glass_hover() -> AdaptiveColor {
    let mut light_hsla: Hsla = rgb(0x111214).into();
    light_hsla.a = 0.11;
    let mut dark_hsla: Hsla = rgb(0xffffff).into();
    dark_hsla.a = 0.14;
    AdaptiveColor::new(light_hsla, dark_hsla)
}

/// Active sidebar navigation pill background (subtle warm translucent highlight).
pub fn sidebar_active_surface() -> AdaptiveColor {
    let mut light_hsla: Hsla = rgb(0x111214).into();
    light_hsla.a = 0.08;
    let mut dark_hsla: Hsla = rgb(0xffffff).into();
    dark_hsla.a = 0.10;
    AdaptiveColor::new(light_hsla, dark_hsla)
}

/// Subtle 1px inner-highlight hairline border for translucent frosted surfaces.
pub fn glass_hairline() -> AdaptiveColor {
    let mut light_hsla: Hsla = rgb(0x000000).into();
    light_hsla.a = 0.08;
    let mut dark_hsla: Hsla = rgb(0xffffff).into();
    dark_hsla.a = 0.09;
    AdaptiveColor::new(light_hsla, dark_hsla)
}

/// Deterministic editorial artwork cover gradient pair `(from, to)` from a seed string.
pub fn cover_gradient_pair(seed: &str) -> (AdaptiveColor, AdaptiveColor) {
    let mut hash: u64 = 14695981039346656037;
    for b in seed.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    type RgbQuad = ([u8; 3], [u8; 3], [u8; 3], [u8; 3]);
    const PAIRS: [RgbQuad; 6] = [
        // Crimson & Deep Berry
        ([235, 94, 110], [160, 32, 66], [156, 36, 54], [52, 18, 34]),
        // Warm Sunset Amber & Terracotta
        ([245, 168, 85], [202, 86, 48], [180, 96, 38], [64, 28, 18]),
        // Midnight Indigo & Electric Violet
        ([124, 134, 245], [88, 66, 188], [58, 72, 164], [24, 26, 68]),
        // Emerald & Teal Lagoon
        ([72, 196, 162], [32, 128, 118], [32, 122, 98], [16, 48, 46]),
        // Magenta & Plum
        (
            [222, 108, 198],
            [134, 48, 164],
            [142, 46, 126],
            [48, 18, 58],
        ),
        // Cobalt & Cyan
        ([92, 168, 240], [44, 96, 192], [38, 92, 164], [16, 36, 68]),
    ];
    let (lf, lt, df, dt) = PAIRS[(hash as usize) % PAIRS.len()];
    (
        AdaptiveColor::new(rgb_u8_to_hsla(lf, 1.0), rgb_u8_to_hsla(df, 1.0)),
        AdaptiveColor::new(rgb_u8_to_hsla(lt, 1.0), rgb_u8_to_hsla(dt, 1.0)),
    )
}

/// High-contrast solid white pill button fill on dark (obsidian fill on light), matching `▶ Play` in ref_4/ref_5.
pub fn color_hero_pill_bg() -> AdaptiveColor {
    AdaptiveColor::new(rgb(0x111214), rgb(0xf5f5f5))
}

pub fn color_hero_pill_fg() -> AdaptiveColor {
    AdaptiveColor::new(rgb(0xffffff), rgb(0x09090b))
}

/// Rich genre card colors for Explore & Moods tiles.
pub fn genre_card_tint(index: usize) -> AdaptiveColor {
    const TINTS: [([u8; 3], [u8; 3]); 6] = [
        ([232, 210, 214], [54, 22, 28]),
        ([214, 222, 244], [24, 34, 62]),
        ([244, 226, 202], [60, 38, 18]),
        ([206, 236, 224], [18, 52, 42]),
        ([234, 214, 242], [48, 24, 60]),
        ([212, 230, 244], [20, 42, 60]),
    ];
    let (l, d) = TINTS[index % TINTS.len()];
    AdaptiveColor::new(rgb_u8_to_hsla(l, 1.0), rgb_u8_to_hsla(d, 1.0))
}

pub fn color_player_surface() -> AdaptiveColor {
    theme().color("player").unwrap_or(theme().card)
}

pub fn color_player_accent() -> AdaptiveColor {
    theme().color("player-accent").unwrap_or(theme().primary)
}

pub fn color_sidebar_surface() -> AdaptiveColor {
    theme().color("sidebar").unwrap_or(theme().card)
}

pub fn color_sidebar_border() -> AdaptiveColor {
    theme().color("sidebar-border").unwrap_or(theme().border)
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
