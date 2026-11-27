//! Truthful Audio DSP Capability Card (`src/screens/settings_dsp_card.rs`).
//!
//! Zero raw color literals, `>= 48dp` mobile touch targets, and honest capability
//! gating: when `AudioDspCapability::ExternalUrlStreamOnly` is active (Wave 1),
//! DSP controls render in a clearly explained disabled/preview state rather than
//! acting as placebo switches. When Wave 2A enables `NativePcmPipeline`, the
//! full 10-band EQ, ReplayGain, Silence Skip, and Crossfade controls unlock.

use std::rc::Rc;

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_core::audio_dsp::DspConfig;

use crate::store;
use crate::store::settings_slice::AudioDspCapability;
use crate::ui::controls::{secondary_button, setting_switch};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_status_ok, glass_hairline, pill_glass_surface, radius_card,
};

#[derive(Clone)]
pub struct DspCardProps {
    pub capability: AudioDspCapability,
    pub dsp_config: DspConfig,
    pub on_toggle_loudness: Rc<dyn Fn(bool)>,
    pub on_toggle_silence_skip: Rc<dyn Fn(bool)>,
    pub on_cycle_eq_preset: Rc<dyn Fn()>,
    pub on_cycle_crossfade: Rc<dyn Fn()>,
}

/// Live-wired DSP settings section reading directly from `crate::store`.
pub fn dsp_settings_section() -> impl IntoElement {
    dynamic(move || {
        dsp_settings_card(DspCardProps {
            capability: store::dsp_capability(),
            dsp_config: store::dsp_config(),
            on_toggle_loudness: Rc::new(store::set_loudness_normalization),
            on_toggle_silence_skip: Rc::new(store::set_silence_skip),
            on_cycle_eq_preset: Rc::new(store::cycle_eq_preset),
            on_cycle_crossfade: Rc::new(store::cycle_crossfade_ms),
        })
        .into_element()
    })
}

/// Pure, testable DSP capability card supporting both gated and active states.
pub fn dsp_settings_card(props: DspCardProps) -> impl IntoElement {
    let is_active = props.capability.is_active();
    let badge_text = if is_active {
        props.capability.badge_label()
    } else {
        "Native PCM Engine Required"
    };
    let detail_text = props.capability.detail_text();

    let eq_label = format!("10-Band EQ: {}", props.dsp_config.eq_preset.label());
    let crossfade_label = if props.dsp_config.crossfade_ms == 0 {
        "Gapless (0s)".to_owned()
    } else {
        format!("Crossfade {}s", props.dsp_config.crossfade_ms / 1000)
    };

    let loudness_val = props.dsp_config.loudness_normalization;
    let silence_val = props.dsp_config.silence_skip_enabled;
    let on_loudness = props.on_toggle_loudness.clone();
    let on_silence = props.on_toggle_silence_skip.clone();
    let on_eq = props.on_cycle_eq_preset.clone();
    let on_crossfade = props.on_cycle_crossfade.clone();

    view()
        .test_id("settings-dsp-card")
        .p_4()
        .gap_3()
        .rounded(radius_card)
        .bg(move || theme().card)
        .border_1()
        .border_color(glass_hairline)
        .child(
            view()
                .gap_1()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            text("Audio DSP & Gapless")
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            view()
                                .px_2p5()
                                .py_1()
                                .rounded_full()
                                .bg(pill_glass_surface)
                                .child(
                                    text(badge_text)
                                        .test_id("audio-dsp-capability-badge")
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(move || {
                                            if is_active {
                                                color_status_ok()
                                            } else {
                                                theme().muted_foreground
                                            }
                                        }),
                                ),
                        ),
                )
                .child(
                    text(detail_text)
                        .test_id("audio-dsp-capability-detail")
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .when(!is_active, {
            let eq_label = eq_label.clone();
            let crossfade_label = crossfade_label.clone();
            move |container| {
                container
                .child(
                    view()
                        .test_id("audio-dsp-gated-notice")
                        .p_3()
                        .rounded(radius_card)
                        .bg(pill_glass_surface)
                        .border_1()
                        .border_color(glass_hairline)
                        .child(
                            text("Direct system playback is active. 10-band EQ, ReplayGain, Silence Skip, and Crossfade activate automatically when the native PCM engine is enabled.")
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(
                    view()
                        .test_id("audio-dsp-preview-controls")
                        .opacity(0.5)
                        .gap_2()
                        .child(gated_preview_row("ReplayGain / EBU R128 Loudness", "Requires PCM"))
                        .child(gated_preview_row("Skip Silence", "Requires PCM"))
                        .child(
                            view()
                                .flex_row()
                                .flex_wrap()
                                .gap_2()
                                .child(gated_preview_chip(&eq_label))
                                .child(gated_preview_chip(&crossfade_label)),
                        ),
                )
            }
        })
        .when(is_active, move |container| {
            let on_loudness = on_loudness.clone();
            let on_silence = on_silence.clone();
            let on_eq = on_eq.clone();
            let on_crossfade = on_crossfade.clone();
            let eq_label = eq_label.clone();
            let crossfade_label = crossfade_label.clone();
            let eq_gains = props.dsp_config.eq_gains_db;
            const BAND_LABELS: [&str; 10] = [
                "31Hz", "62Hz", "125Hz", "250Hz", "500Hz", "1kHz", "2kHz", "4kHz", "8kHz", "16kHz",
            ];

            container
                .child(setting_switch(
                    "ReplayGain / EBU R128 Loudness",
                    move || loudness_val,
                    move |next| on_loudness(next),
                ))
                .child(setting_switch(
                    "Skip Silence",
                    move || silence_val,
                    move |next| on_silence(next),
                ))
                .child(
                    view()
                        .test_id("audio-dsp-active-controls")
                        .flex_row()
                        .flex_wrap()
                        .gap_2()
                        .child(secondary_button(eq_label, move || on_eq()))
                        .child(secondary_button("Reset EQ Flat", store::reset_eq_flat))
                        .child(secondary_button(crossfade_label, move || on_crossfade())),
                )
                .child(
                    view()
                        .test_id("audio-dsp-eq-bands")
                        .flex_row()
                        .flex_wrap()
                        .gap_2()
                        .children((0..10).map(move |idx| {
                            let gain = eq_gains[idx];
                            let lbl = BAND_LABELS[idx];
                            let gain_txt = if gain >= 0.0 {
                                format!("+{gain:.0}dB")
                            } else {
                                format!("{gain:.0}dB")
                            };
                            view()
                                .min_w(px(76.0))
                                .p_2()
                                .gap_1()
                                .items_center()
                                .rounded(radius_card)
                                .bg(pill_glass_surface)
                                .border_1()
                                .border_color(glass_hairline)
                                .child(
                                    text(lbl)
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(move || theme().muted_foreground),
                                )
                                .child(
                                    text(gain_txt)
                                        .text_xs()
                                        .tabular_nums()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .gap_1()
                                        .child(
                                            button()
                                                .min_w(px(MOBILE_MIN_TOUCH_TARGET))
                                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                                .items_center()
                                                .justify_center()
                                                .rounded(move || theme().radius)
                                                .bg(move || theme().card)
                                                .on_press(move || {
                                                    store::set_eq_band_gain(idx, gain - 1.5);
                                                })
                                                .child(
                                                    text("-")
                                                        .text_sm()
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(move || theme().foreground),
                                                ),
                                        )
                                        .child(
                                            button()
                                                .min_w(px(MOBILE_MIN_TOUCH_TARGET))
                                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                                .items_center()
                                                .justify_center()
                                                .rounded(move || theme().radius)
                                                .bg(move || theme().card)
                                                .on_press(move || {
                                                    store::set_eq_band_gain(idx, gain + 1.5);
                                                })
                                                .child(
                                                    text("+")
                                                        .text_sm()
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(move || theme().foreground),
                                                ),
                                        ),
                                )
                        })),
                )
        })
}

fn gated_preview_row(label: &'static str, status: &'static str) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_4()
        .py_2()
        .rounded(radius_card)
        .bg(move || theme().card)
        .border_1()
        .border_color(glass_hairline)
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().muted_foreground),
        )
        .child(
            text(status)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
}

fn gated_preview_chip(label: &str) -> impl IntoElement {
    button()
        .disabled(true)
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_4()
        .items_center()
        .justify_center()
        .rounded(move || theme().radius)
        .bg(pill_glass_surface)
        .border_1()
        .border_color(glass_hairline)
        .child(
            text(label.to_owned())
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().muted_foreground),
        )
}
