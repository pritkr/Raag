//! GPU shader backdrops, live equalizer bars, and fluid transport micro-animations.
//!
//! Zero raw color literals; zero `backdrop_blur` (safe inside virtualized lists).
//! All shaders & animations respect `store::reduced_motion()` and pause when audio is idle.

use std::sync::OnceLock;

use nsh_core::prelude::*;
use nsh_core::shader::{Shader, UniformValue};

use crate::store;
use crate::ui::icons::IconKind;
use crate::ui::tokens::{
    ambient_backdrop_color, ambient_shader_palette_triple, color_player_accent, motion,
};

const AMBIENT_DRIFT_WGSL: &str = r#"
struct Uniforms {
    prev_c1: vec4f,
    prev_c2: vec4f,
    prev_c3: vec4f,
    next_c1: vec4f,
    next_c2: vec4f,
    next_c3: vec4f,
    mix_t: f32,
};

fn shade(uv: vec2f) -> vec4f {
    let t = clamp(uniforms.mix_t, 0.0, 1.0);
    let c1 = mix(uniforms.prev_c1.rgb, uniforms.next_c1.rgb, t);
    let c2 = mix(uniforms.prev_c2.rgb, uniforms.next_c2.rgb, t);
    let c3 = mix(uniforms.prev_c3.rgb, uniforms.next_c3.rgb, t);

    let aspect = vec2f(max(nsh.size.x / max(nsh.size.y, 1.0), 0.5), 1.0);
    let p = (uv - vec2f(0.5, 0.5)) * aspect;

    // Slow, organic fluid drift of layered radial blobs across the entire shell
    let phase = nsh.time * 0.11;
    let center1 = vec2f(-0.24 + sin(phase * 0.63) * 0.28, -0.18 + cos(phase * 0.47) * 0.24) * aspect;
    let center2 = vec2f(0.28 + cos(phase * 0.53 + 2.1) * 0.30, 0.14 + sin(phase * 0.41 + 1.4) * 0.28) * aspect;
    let center3 = vec2f(-0.08 + sin(phase * 0.37 + 4.0) * 0.34, 0.28 + cos(phase * 0.59 + 3.2) * 0.22) * aspect;
    let center4 = vec2f(0.18 + cos(phase * 0.29 + 0.9) * 0.26, -0.26 + sin(phase * 0.51 + 2.6) * 0.20) * aspect;

    let d1 = length(p - center1);
    let d2 = length(p - center2);
    let d3 = length(p - center3);
    let d4 = length(p - center4);

    let w1 = exp(-d1 * d1 * 1.45) * 0.68;
    let w2 = exp(-d2 * d2 * 1.55) * 0.56;
    let w3 = exp(-d3 * d3 * 1.85) * 0.42;
    let w4 = exp(-d4 * d4 * 2.10) * 0.34;

    var col = c1;
    col = mix(col, c2, clamp(w1, 0.0, 0.68));
    col = mix(col, c3, clamp(w2, 0.0, 0.56));
    col = mix(col, mix(c2, c3, 0.55), clamp(w3, 0.0, 0.42));
    col = mix(col, mix(c1, c2, 0.65), clamp(w4, 0.0, 0.34));

    // Soft frosted-glass edge vignette so outer chrome stays calm and legible
    let edge = clamp(length(uv - vec2f(0.48, 0.46)) * 1.12, 0.0, 1.0);
    col = mix(col, c1 * 0.78, edge * edge * 0.45);

    return vec4f(clamp(col, vec3f(0.0), vec3f(1.0)), 1.0);
}
"#;

const EQ_BARS_WGSL: &str = r#"
struct Uniforms {
    accent: vec4f,
    active: f32,
};

fn bar_mask(uv: vec2f, center_x: f32, half_w: f32, height: f32) -> f32 {
    let dx = abs(uv.x - center_x) - half_w;
    let top_y = 1.0 - height;
    let in_x = 1.0 - smoothstep(0.0, 0.04, dx);
    let in_y = smoothstep(top_y - 0.04, top_y + 0.02, uv.y) * (1.0 - smoothstep(0.94, 1.0, uv.y));
    return in_x * in_y;
}

fn shade(uv: vec2f) -> vec4f {
    let t = nsh.time * 4.8;
    let play = clamp(uniforms.active, 0.0, 1.0);

    let h1 = mix(0.32, 0.24 + 0.66 * (0.5 + 0.5 * sin(t * 1.05 + 0.2)), play);
    let h2 = mix(0.58, 0.30 + 0.66 * (0.5 + 0.5 * sin(t * 1.37 + 2.1)), play);
    let h3 = mix(0.40, 0.22 + 0.64 * (0.5 + 0.5 * sin(t * 0.91 + 4.3)), play);

    let m1 = bar_mask(uv, 0.22, 0.095, h1);
    let m2 = bar_mask(uv, 0.50, 0.095, h2);
    let m3 = bar_mask(uv, 0.78, 0.095, h3);

    let alpha = clamp(max(m1, max(m2, m3)), 0.0, 1.0);
    return vec4f(uniforms.accent.rgb, alpha * uniforms.accent.a);
}
"#;

fn ambient_drift_shader() -> Option<Shader> {
    static COMPILED: OnceLock<Option<Shader>> = OnceLock::new();
    COMPILED
        .get_or_init(|| Shader::compile(AMBIENT_DRIFT_WGSL).ok())
        .clone()
}

fn eq_bars_shader() -> Option<Shader> {
    static COMPILED: OnceLock<Option<Shader>> = OnceLock::new();
    COMPILED
        .get_or_init(|| Shader::compile(EQ_BARS_WGSL).ok())
        .clone()
}

/// Thread-safe snapshot of the active track's calibrated ambient shader palette for preview hosts.
#[derive(Debug, Clone, Copy)]
pub struct AmbientBackdropSnapshot {
    pub dark_base: [u8; 3],
    pub dark_accent: [u8; 3],
    pub light_base: [u8; 3],
    pub light_accent: [u8; 3],
    pub reduced_motion: bool,
    pub is_playing: bool,
}

impl Default for AmbientBackdropSnapshot {
    fn default() -> Self {
        let p = resonance_core::palette::AmbientPalette::default();
        Self {
            dark_base: p.dark_backdrop,
            dark_accent: p.dark_accent,
            light_base: p.light_backdrop,
            light_accent: p.light_accent,
            reduced_motion: false,
            is_playing: true,
        }
    }
}

fn ambient_mirror() -> &'static std::sync::RwLock<AmbientBackdropSnapshot> {
    static MIRROR: OnceLock<std::sync::RwLock<AmbientBackdropSnapshot>> = OnceLock::new();
    MIRROR.get_or_init(|| std::sync::RwLock::new(AmbientBackdropSnapshot::default()))
}

pub fn latest_ambient_snapshot() -> AmbientBackdropSnapshot {
    ambient_mirror()
        .read()
        .map(|g| *g)
        .unwrap_or_default()
}

fn update_ambient_mirror(
    palette: resonance_core::palette::AmbientPalette,
    reduced_motion: bool,
    is_playing: bool,
) {
    if let Ok(mut guard) = ambient_mirror().write() {
        *guard = AmbientBackdropSnapshot {
            dark_base: palette.dark_backdrop,
            dark_accent: palette.dark_accent,
            light_base: palette.light_backdrop,
            light_accent: palette.light_accent,
            reduced_motion,
            is_playing,
        };
    }
}

fn ambient_shader_layer(
    test_id_name: &'static str,
    always_drift_when_allowed: bool,
) -> impl IntoElement {
    let initial_palette = store::active_ambient_palette();
    update_ambient_mirror(
        initial_palette,
        store::reduced_motion(),
        store::player_snapshot().state.is_playing(),
    );
    let initial = ambient_shader_palette_triple(initial_palette);
    let prev_colors = signal(initial);
    let next_colors = signal(initial);
    let blend = animated_value(1.0);

    let blend_effect = blend.clone();
    effect(move || {
        let palette = store::active_ambient_palette();
        let reduced = store::reduced_motion();
        let playing = store::player_snapshot().state.is_playing();
        update_ambient_mirror(palette, reduced, playing);
        let target = ambient_shader_palette_triple(palette);
        let current = next_colors.get_untracked();
        if current != target {
            prev_colors.set(current);
            next_colors.set(target);
            let fade_ms = motion::transition_ms(motion::PALETTE_FADE_MS, reduced);
            if fade_ms <= 0.0 {
                blend_effect.set(1.0);
            } else {
                blend_effect.set(0.0);
                blend_effect.timing(1.0, fade_ms, Easing::EaseInOut);
            }
        }
    });

    let maybe_shader = ambient_drift_shader();
    let has_shader = maybe_shader.is_some();

    view()
        .test_id(test_id_name)
        .absolute()
        .inset_0()
        .pointer_events(PointerEvents::None)
        .accessibility_elements_hidden(true)
        .bg(move || ambient_backdrop_color(store::active_ambient_palette()))
        .child(when(
            move || shaders_available() && has_shader,
            {
                let shader_handle = maybe_shader.clone();
                let blend_uniform = blend.clone();
                move || {
                    let program = shader_handle.clone().expect("checked is_some");
                    shader()
                        .source(program)
                        .size_full()
                        .paused(move || {
                            store::reduced_motion()
                                || (!always_drift_when_allowed
                                    && !store::player_snapshot().state.is_playing())
                        })
                        .uniform("prev_c1", move || UniformValue::from(prev_colors.get().0))
                        .uniform("prev_c2", move || UniformValue::from(prev_colors.get().1))
                        .uniform("prev_c3", move || UniformValue::from(prev_colors.get().2))
                        .uniform("next_c1", move || UniformValue::from(next_colors.get().0))
                        .uniform("next_c2", move || UniformValue::from(next_colors.get().1))
                        .uniform("next_c3", move || UniformValue::from(next_colors.get().2))
                        .uniform("mix_t", &blend_uniform)
                }
            },
        ))
}

/// Full-window drifting blurred-glass ambient background mounted once behind the entire app shell
/// (sidebar, top bar, main content, queue drawer, and bottom player bar). Never mounted inside
/// virtualized lists, and honors `motion::duration_ms` / `store::reduced_motion()`.
pub fn ambient_shell_backdrop() -> impl IntoElement {
    ambient_shader_layer("ambient-shell-backdrop", true)
}

/// Full-bleed artwork-driven moving ambient shader backdrop for Now Playing & Synced Lyrics,
/// with smooth palette cross-fade and reduced-motion pausing.
pub fn ambient_shader_backdrop() -> impl IntoElement {
    ambient_shader_layer("ambient-now-playing-backdrop", false)
}

/// Minimal 3-bar animated playing equalizer indicator (GPU shader with static themed bar fallback).
/// Automatically pauses when audio is paused or reduced-motion is active. Zero blur — safe in virtual lists.
pub fn live_equalizer_indicator(
    is_playing: impl Fn() -> bool + Copy + 'static,
    size_px: f32,
) -> impl IntoElement {
    let maybe_shader = eq_bars_shader();
    let has_shader = maybe_shader.is_some();

    view()
        .w(px(size_px))
        .h(px(size_px))
        .items_center()
        .justify_center()
        .pointer_events(PointerEvents::None)
        .accessibility_elements_hidden(true)
        .child(
            when(
                move || shaders_available() && has_shader,
                {
                    let shader_handle = maybe_shader.clone();
                    move || {
                        let program = shader_handle.clone().expect("checked is_some");
                        shader()
                            .source(program)
                            .w(px(size_px))
                            .h(px(size_px))
                            .paused(move || !is_playing() || store::reduced_motion())
                            .uniform("accent", move || {
                                UniformValue::from(color(color_player_accent()))
                            })
                            .uniform("active", move || {
                                UniformValue::from(
                                    if is_playing() && !store::reduced_motion() {
                                        1.0_f32
                                    } else {
                                        0.0_f32
                                    },
                                )
                            })
                    }
                },
            )
            .otherwise(move || {
                let bar_w = (size_px * 0.19).max(2.5);
                let gap = (size_px * 0.12).max(2.0);
                view()
                    .w(px(size_px))
                    .h(px(size_px))
                    .flex_row()
                    .items_end()
                    .justify_center()
                    .gap(px(gap))
                    .py_0p5()
                    .child(
                        view()
                            .w(px(bar_w))
                            .h(move || {
                                px(size_px * if is_playing() { 0.56 } else { 0.34 })
                            })
                            .rounded_full()
                            .bg(color_player_accent),
                    )
                    .child(
                        view()
                            .w(px(bar_w))
                            .h(move || {
                                px(size_px * if is_playing() { 0.88 } else { 0.54 })
                            })
                            .rounded_full()
                            .bg(color_player_accent),
                    )
                    .child(
                        view()
                            .w(px(bar_w))
                            .h(move || {
                                px(size_px * if is_playing() { 0.44 } else { 0.38 })
                            })
                            .rounded_full()
                            .bg(color_player_accent),
                    )
            }),
        )
}

/// Tactile scaling Play <-> Pause icon swap (240ms ease-out, reduced-motion gated).
pub fn animated_play_pause_icon(
    is_playing: impl Fn() -> bool + 'static,
    size_px: f32,
    color_fn: impl Fn() -> AdaptiveColor + Copy + 'static,
) -> impl IntoElement {
    let is_playing = std::rc::Rc::new(is_playing);
    let scale = animated_value(1.0);
    let was_playing = signal(is_playing());
    let scale_effect = scale.clone();
    let is_playing_effect = is_playing.clone();
    effect(move || {
        let now = is_playing_effect();
        let prev = was_playing.get_untracked();
        if now != prev {
            was_playing.set(now);
            if !store::reduced_motion() {
                scale_effect.set(0.82);
                scale_effect.timing(1.0, motion::SWAP_MS, Easing::EaseOut);
            } else {
                scale_effect.set(1.0);
            }
        }
    });

    view()
        .w(px(size_px))
        .h(px(size_px))
        .items_center()
        .justify_center()
        .bind_scale(&scale)
        .child(crate::ui::icons::reactive_icon(
            move || {
                if is_playing() {
                    IconKind::Pause
                } else {
                    IconKind::Play
                }
            },
            size_px,
            color_fn,
        ))
}

/// Heart icon with a subtle tactile spring pop when transitioning to Liked (reduced-motion gated).
pub fn animated_like_heart(
    is_liked: impl Fn() -> bool + Copy + 'static,
    size_px: f32,
    active_color_fn: impl Fn() -> AdaptiveColor + Copy + 'static,
    inactive_color_fn: impl Fn() -> AdaptiveColor + Copy + 'static,
) -> impl IntoElement {
    let scale = animated_value(1.0);
    let was_liked = signal(is_liked());

    let scale_effect = scale.clone();
    effect(move || {
        let now = is_liked();
        let prev = was_liked.get_untracked();
        if now != prev {
            was_liked.set(now);
            if !store::reduced_motion() {
                if now {
                    scale_effect.set(0.72);
                    scale_effect.spring(
                        1.0,
                        Spring {
                            stiffness: 380.0,
                            damping: 13.0,
                            ..Spring::default()
                        },
                    );
                } else {
                    scale_effect.set(0.88);
                    scale_effect.timing(1.0, motion::PRESS_MS, Easing::EaseOut);
                }
            } else {
                scale_effect.set(1.0);
            }
        }
    });

    view()
        .w(px(size_px))
        .h(px(size_px))
        .items_center()
        .justify_center()
        .bind_scale(&scale)
        .child(crate::ui::icons::reactive_icon(
            move || {
                if is_liked() {
                    IconKind::HeartFilled
                } else {
                    IconKind::Heart
                }
            },
            size_px,
            move || {
                if is_liked() {
                    active_color_fn()
                } else {
                    inactive_color_fn()
                }
            },
        ))
}
