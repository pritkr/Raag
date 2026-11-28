//! Semantic theme, glass, motion, and typography tokens reading from `theme.json` via `nsh_base::theme()`.
//!
//! Zero raw hex/RGB values in presentation code (`src/screens/`, `src/ui/controls/`).
//! All static glass, pill, cover-gradient, and genre-tint colors live in `theme.json`.

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

/// Compact narrow-mobile threshold (320px–379px) for responsive reflow & artwork scaling.
pub const NARROW_VIEWPORT_MAX_WIDTH: f32 = 380.0;

/// Optimal prose line-length cap (~60–72ch at 14–16px body size).
pub const PROSE_MAX_WIDTH: f32 = 660.0;

/// Motion tokens honoring system reduced-motion setting.
pub mod motion {
    use super::*;

    pub const FAST_MS: u64 = 120;
    pub const NORMAL_MS: u64 = 180;
    pub const SPRING_MS: u64 = 240;

    /// Micro-interaction press & hover transition duration (ms).
    pub const PRESS_MS: f32 = 150.0;
    /// Play/Pause icon cross-fade & scale swap duration (ms).
    pub const SWAP_MS: f32 = 240.0;
    /// Sidebar / side-panel collapse & sliding indicator duration (ms).
    pub const LAYOUT_MS: f32 = 260.0;
    /// Artwork-driven ambient shader palette cross-fade duration (ms).
    pub const PALETTE_FADE_MS: f32 = 500.0;
    /// Tactile button press scale (gated by `!store::reduced_motion()`).
    pub const PRESS_SCALE: f32 = 0.96;

    pub fn duration_ms(base_ms: u64, reduced_motion: bool) -> Duration {
        if reduced_motion {
            Duration::from_millis(0)
        } else {
            Duration::from_millis(base_ms)
        }
    }

    pub fn transition_ms(base_ms: f32, reduced_motion: bool) -> f32 {
        if reduced_motion { 0.0 } else { base_ms }
    }
}

pub fn is_desktop_layout() -> bool {
    window().width.0 >= DESKTOP_MIN_WIDTH
}

/// Reactive breakpoint helper matching `is_desktop_layout()`.
pub fn breakpoint_is_desktop() -> bool {
    window().width.0 >= DESKTOP_MIN_WIDTH
}

/// Responsive grid column count based on current window width.
pub fn grid_columns() -> u32 {
    let w = window().width.0;
    if w >= 1280.0 {
        5
    } else if w >= 960.0 {
        4
    } else if w >= 640.0 {
        3
    } else {
        2
    }
}

pub fn is_narrow_viewport() -> bool {
    window().width.0 < NARROW_VIEWPORT_MAX_WIDTH
}

pub fn is_desktop_panel_visible() -> bool {
    window().width.0 >= DESKTOP_PANEL_MIN_WIDTH
}

pub fn radius_card() -> Points {
    px(theme().number("radius-card").unwrap_or(12.0))
}

pub fn radius_inset() -> Points {
    px(theme().number("radius-inset").unwrap_or(8.0))
}

/// Concentric inner radius for elements nested inside a `radius_panel` container (`18px - 10px - 1px = 7px`).
pub fn radius_panel_inner() -> Points {
    px((theme().number("radius-panel").unwrap_or(18.0) - 11.0).max(4.0))
}

/// Concentric inner radius for tabs/segments nested inside a `radius_card` container (`12px - 4px = 8px`).
pub fn radius_tab_inner() -> Points {
    radius_inset()
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

/// Resolves an artwork `AmbientPalette` into a 3-color harmonic HSLA triple for the GPU ambient
/// drift shader under the currently active color scheme (`c1` base, `c2` primary bloom, `c3` complementary drift).
pub fn ambient_shader_palette_triple(palette: AmbientPalette) -> (Hsla, Hsla, Hsla) {
    let base = color(ambient_backdrop_color(palette));
    let accent = color(ambient_accent_color(palette));
    let mut tertiary = accent;
    // Complementary warm/cool shift (gives the iconic amber + deep teal / crimson + indigo ambient depth)
    tertiary.h = (tertiary.h + 0.42).fract();
    tertiary.s = (tertiary.s * 0.88).clamp(0.14, 0.82);
    tertiary.l = ((base.l * 0.48) + (accent.l * 0.52)).clamp(0.06, 0.94);
    tertiary.a = 1.0;

    let mut c1 = base;
    c1.a = 1.0;
    let mut c2 = accent;
    c2.a = 1.0;
    (c1, c2, tertiary)
}

/// Active track accent tint for the thin full-width bottom player progress bar.
pub fn ambient_progress_color(palette: AmbientPalette) -> AdaptiveColor {
    let boost = |c: [u8; 3], light_mode: bool| -> [u8; 3] {
        if light_mode {
            // Deepen slightly so progress stands out cleanly in Light mode
            [
                (c[0] as f32 * 0.62) as u8,
                (c[1] as f32 * 0.62) as u8,
                (c[2] as f32 * 0.62) as u8,
            ]
        } else {
            // Brighten toward crisp luminous white-tinted accent in Dark mode
            [
                ((c[0] as f32 * 0.35) + 165.0).min(255.0) as u8,
                ((c[1] as f32 * 0.35) + 165.0).min(255.0) as u8,
                ((c[2] as f32 * 0.35) + 165.0).min(255.0) as u8,
            ]
        }
    };
    AdaptiveColor::new(
        rgb_u8_to_hsla(boost(palette.light_accent, true), 1.0),
        rgb_u8_to_hsla(boost(palette.dark_accent, false), 1.0),
    )
}

/// Subtle warm top-ambient tint matching the reference YouTube Music Desktop canvas glow.
pub fn color_ambient_warm() -> AdaptiveColor {
    theme().color("ambient-warm").unwrap_or(theme().accent)
}

/// Calibrated translucent frosted glass surface for Desktop Player Bar, Mini-Player & Bottom Nav.
pub fn frosted_bar_surface() -> AdaptiveColor {
    theme()
        .color("glass-bar")
        .unwrap_or(theme().card)
        .opacity(0.62)
}

/// Calibrated translucent frosted glass surface for the Left Sidebar over the moving ambient backdrop.
pub fn frosted_sidebar_surface() -> AdaptiveColor {
    theme()
        .color("glass-sidebar")
        .unwrap_or(theme().card)
        .opacity(0.28)
}

/// Calibrated translucent frosted glass surface for the Right `Up next` Queue / Lyrics drawer.
pub fn frosted_drawer_surface() -> AdaptiveColor {
    theme()
        .color("glass-drawer")
        .unwrap_or(theme().card)
        .opacity(0.54)
}

/// Calibrated translucent frosted glass surface for the sticky Desktop Top Header.
pub fn frosted_header_surface() -> AdaptiveColor {
    theme()
        .color("glass-header")
        .unwrap_or(theme().background)
        .opacity(0.16)
}

/// Calibrated translucent glass card surface for Settings cards, Listen Together cards, and Search Top Result.
pub fn frosted_settings_card() -> AdaptiveColor {
    AdaptiveColor::new(
        color(theme().card).opacity(0.84),
        color(
            theme()
                .color("glass-settings-card")
                .unwrap_or(theme().foreground),
        )
        .opacity(0.065),
    )
}

/// Calibrated translucent glass card surface for Now Playing control dock, modals, and popovers.
pub fn frosted_card_surface() -> AdaptiveColor {
    AdaptiveColor::new(
        color(theme().card).opacity(0.88),
        color(theme().color("glass-card").unwrap_or(theme().foreground)).opacity(0.085),
    )
}

/// Soft dimmed overlay scrim behind glassmorphism modals and bottom sheets.
pub fn overlay_scrim() -> AdaptiveColor {
    theme()
        .color("overlay-scrim")
        .unwrap_or(theme().background)
        .opacity(0.56)
}

/// Subtle translucent pill surface for search bar, mood chips, and action pills.
pub fn pill_glass_surface() -> AdaptiveColor {
    theme()
        .color("glass-pill")
        .unwrap_or(theme().foreground)
        .opacity(0.08)
}

/// Hover state for translucent pills.
pub fn pill_glass_hover() -> AdaptiveColor {
    theme()
        .color("glass-pill")
        .unwrap_or(theme().foreground)
        .opacity(0.15)
}

/// Active sidebar navigation pill background (subtle translucent glass highlight).
pub fn sidebar_active_surface() -> AdaptiveColor {
    theme()
        .color("glass-pill")
        .unwrap_or(theme().foreground)
        .opacity(0.12)
}

/// Subtle 1px inner-highlight hairline border for translucent frosted surfaces.
pub fn glass_hairline() -> AdaptiveColor {
    theme()
        .color("glass-hairline")
        .unwrap_or(theme().border)
        .opacity(0.10)
}

/// Subtle 1px top specular highlight hairline for elevated glass docks and sheets.
pub fn glass_specular_hairline() -> AdaptiveColor {
    theme()
        .color("glass-specular")
        .unwrap_or(theme().foreground)
        .opacity(0.16)
}

/// Deterministic editorial artwork cover gradient pair `(from, to)` resolved from `theme.json`.
pub fn cover_gradient_pair(seed: &str) -> (AdaptiveColor, AdaptiveColor) {
    let mut hash: u64 = 14695981039346656037;
    for b in seed.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    const KEYS: [(&str, &str); 6] = [
        ("cover-1-from", "cover-1-to"),
        ("cover-2-from", "cover-2-to"),
        ("cover-3-from", "cover-3-to"),
        ("cover-4-from", "cover-4-to"),
        ("cover-5-from", "cover-5-to"),
        ("cover-6-from", "cover-6-to"),
    ];
    let (from_key, to_key) = KEYS[(hash as usize) % KEYS.len()];
    let t = theme();
    (
        t.color(from_key).unwrap_or(t.primary),
        t.color(to_key).unwrap_or(t.accent),
    )
}

/// High-contrast solid pill button fill (`Play` / floating card play pill), sourced from `theme.json`.
pub fn color_hero_pill_bg() -> AdaptiveColor {
    theme().color("hero-pill").unwrap_or(theme().foreground)
}

pub fn color_hero_pill_fg() -> AdaptiveColor {
    theme()
        .color("hero-pill-foreground")
        .unwrap_or(theme().background)
}

/// Rich genre card colors for Explore & Moods tiles, sourced from `theme.json`.
pub fn genre_card_tint(index: usize) -> AdaptiveColor {
    const KEYS: [&str; 6] = [
        "genre-1", "genre-2", "genre-3", "genre-4", "genre-5", "genre-6",
    ];
    let t = theme();
    t.color(KEYS[index % KEYS.len()]).unwrap_or(t.secondary)
}

/// Vibrant left vertical accent stripe color for `Moods & genres` tiles (matches `02_explore_02.png`).
pub fn genre_stripe_color(index: usize) -> AdaptiveColor {
    const KEYS: [&str; 6] = [
        "stripe-1", "stripe-2", "stripe-3", "stripe-4", "stripe-5", "stripe-6",
    ];
    let t = theme();
    t.color(KEYS[index % KEYS.len()]).unwrap_or(t.primary)
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

/// High-contrast text/icon foreground painted on `theme().destructive` fills (`>= 5.9:1` Light & Dark).
pub fn color_destructive_foreground() -> AdaptiveColor {
    theme()
        .color("destructive-foreground")
        .unwrap_or(theme().primary_foreground)
}

/// Guaranteed white (`#ffffff`) foreground for monograms & badges painted over cover gradients.
pub fn color_on_cover() -> AdaptiveColor {
    theme().color("on-cover").unwrap_or(theme().foreground)
}

// ---------------------------------------------------------------------------
// Semantic Typography Helpers (replaces ad-hoc ALL-CAPS strings & repeated styles)
// ---------------------------------------------------------------------------

/// Uppercase tracked eyebrow / section kicker label (semantic Title Case input rendered uppercase).
pub fn eyebrow_label(label: impl IntoReactive<SharedString>) -> impl IntoElement {
    text(label)
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .uppercase()
        .letter_spacing(px(0.9))
        .text_color(move || theme().muted_foreground)
}

/// Primary shelf or screen section heading.
pub fn section_title_label(label: impl IntoReactive<SharedString>) -> impl IntoElement {
    text(label)
        .text_lg()
        .font_weight(FontWeight::BOLD)
        .text_color(move || theme().foreground)
}

/// Single-line truncated card / row title.
pub fn card_title_label(label: impl IntoReactive<SharedString>) -> impl IntoElement {
    text(label)
        .number_of_lines(1)
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || theme().foreground)
}

/// Muted secondary caption / metadata label.
pub fn muted_caption_label(label: impl IntoReactive<SharedString>) -> impl IntoElement {
    text(label)
        .number_of_lines(1)
        .text_xs()
        .text_color(move || theme().muted_foreground)
}
