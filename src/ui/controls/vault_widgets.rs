//! Reusable Offline Vault UI controls: Pin Offline button & Storage/Quota settings card.
//! Zero raw color literals — semantic theme tokens only.

use nsh_base::{alert_dialog, button, theme};
use nsh_core::prelude::*;
use resonance_core::offline::{CollectionOfflineProgress, QUOTA_PRESETS_BYTES};

use crate::ui::controls::primitives::setting_switch;
use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_player_accent, glass_hairline, pill_glass_hover,
    pill_glass_surface, radius_card, radius_pill,
};

/// 3-state Collection Pin Offline button (`Pin Offline`, `Downloading (4/10)`, `Offline · Tap to Unpin`).
pub fn collection_pin_button(
    title: String,
    progress: CollectionOfflineProgress,
    on_pin: impl Fn() + 'static,
    on_unpin: impl Fn() + 'static,
) -> impl IntoElement {
    let is_pinned = progress.is_pinned();
    let label = progress.format_badge();
    let icon_kind = match &progress {
        CollectionOfflineProgress::Unpinned => IconKind::Download,
        CollectionOfflineProgress::Complete { .. } => IconKind::CheckCircle,
        CollectionOfflineProgress::Syncing { .. } => IconKind::Pause,
        CollectionOfflineProgress::Paused { .. } => IconKind::Download,
        CollectionOfflineProgress::PartialFailed { .. } => IconKind::AlertCircle,
    };

    let on_unpin_rc = std::rc::Rc::new(on_unpin);
    let title_for_dialog = title;

    button()
        .test_id("collection-pin-offline-btn")
        .on_press(move || {
            if is_pinned {
                let cb = on_unpin_rc.clone();
                let t = title_for_dialog.clone();
                spawn(async move {
                    let answer = alert_dialog()
                        .title("Remove offline downloads?")
                        .description(format!(
                            "Remove \"{t}\" from offline storage? Tracks used by other playlists will be kept."
                        ))
                        .cancel("cancel", "Keep")
                        .destructive("remove", "Remove")
                        .show()
                        .await;
                    if matches!(answer, Ok(r) if r.action.as_deref() == Some("remove")) {
                        cb();
                    }
                });
            } else {
                on_pin();
            }
        })
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_4()
        .flex_row()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(radius_pill)
        .bg(move || {
            if is_pinned {
                theme().muted
            } else {
                pill_glass_surface()
            }
        })
        .border_1()
        .border_color(glass_hairline)
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.opacity(0.78))
        .child(icon(icon_kind, 15.0, move || {
            if is_pinned {
                color_player_accent()
            } else {
                theme().foreground
            }
        }))
        .child(
            text(label)
                .text_xs()
                .tabular_nums()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
}

#[derive(Clone)]
pub struct OfflineStorageSettingsProps {
    pub vault_bytes: u64,
    pub cache_bytes: u64,
    pub quota_bytes: u64,
    pub wifi_only: bool,
    pub keep_liked_offline: bool,
    pub quality_label: String,
    pub on_select_quota: std::sync::Arc<dyn Fn(u64) + Send + Sync>,
    pub on_toggle_wifi_only: std::sync::Arc<dyn Fn(bool) + Send + Sync>,
    pub on_toggle_keep_liked: std::sync::Arc<dyn Fn(bool) + Send + Sync>,
    pub on_cycle_quality: std::sync::Arc<dyn Fn() + Send + Sync>,
    pub on_verify_files: std::sync::Arc<dyn Fn() + Send + Sync>,
}

fn format_gb_mb(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else {
        format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Reusable Offline Storage & Vault Settings Card for `settings.rs`.
pub fn offline_storage_settings_card(props: OfflineStorageSettingsProps) -> impl IntoElement {
    let total_budget = if props.quota_bytes > 0 {
        props.quota_bytes
    } else {
        (props.vault_bytes + props.cache_bytes).max(1024 * 1024 * 1024)
    };
    let vault_frac = (props.vault_bytes as f32 / total_budget as f32).clamp(0.0, 1.0);
    let cache_frac = (props.cache_bytes as f32 / total_budget as f32).clamp(0.0, 1.0 - vault_frac);

    let on_wifi = props.on_toggle_wifi_only.clone();
    let on_liked = props.on_toggle_keep_liked.clone();
    let on_quality = props.on_cycle_quality.clone();
    let on_verify = props.on_verify_files.clone();

    view()
        .test_id("offline-storage-settings-card")
        .p_4()
        .gap_3()
        .rounded(radius_card)
        .bg(move || theme().card)
        .border_1()
        .border_color(glass_hairline)
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    text("Offline Storage")
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(format!(
                        "{} downloads • {} cache",
                        format_gb_mb(props.vault_bytes),
                        format_gb_mb(props.cache_bytes)
                    ))
                    .text_xs()
                    .tabular_nums()
                    .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            view()
                .w_full()
                .h(px(8.0))
                .flex_row()
                .rounded_full()
                .bg(move || theme().muted)
                .overflow_hidden()
                .child(
                    view()
                        .h_full()
                        .w(percentage(vault_frac))
                        .bg(move || theme().primary),
                )
                .child(
                    view()
                        .h_full()
                        .w(percentage(cache_frac))
                        .bg(move || theme().accent),
                ),
        )
        .child(view().flex_row().flex_wrap().gap_1p5().children(
            QUOTA_PRESETS_BYTES.into_iter().map(move |(bytes, label)| {
                let selected = props.quota_bytes == bytes;
                let cb = props.on_select_quota.clone();
                button()
                    .on_press(move || cb(bytes))
                    .min_h(px(36.0))
                    .px_3()
                    .items_center()
                    .justify_center()
                    .rounded(radius_pill)
                    .bg(move || {
                        if selected {
                            theme().primary
                        } else {
                            pill_glass_surface()
                        }
                    })
                    .border_1()
                    .border_color(glass_hairline)
                    .child(
                        text(label)
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || {
                                if selected {
                                    theme().primary_foreground
                                } else {
                                    theme().foreground
                                }
                            }),
                    )
            }),
        ))
        .child(setting_switch(
            "Download on Wi-Fi only",
            move || props.wifi_only,
            move |v| on_wifi(v),
        ))
        .child(setting_switch(
            "Keep Liked Songs Offline",
            move || props.keep_liked_offline,
            move |v| on_liked(v),
        ))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(
                    button()
                        .on_press(move || on_quality())
                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                        .px_4()
                        .items_center()
                        .justify_center()
                        .rounded(radius_pill)
                        .bg(pill_glass_surface)
                        .border_1()
                        .border_color(glass_hairline)
                        .child(
                            text(format!("Audio Quality: {}", props.quality_label))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || theme().foreground),
                        ),
                )
                .child(
                    button()
                        .test_id("verify-offline-files-btn")
                        .on_press(move || on_verify())
                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                        .px_4()
                        .items_center()
                        .justify_center()
                        .rounded(radius_pill)
                        .bg(pill_glass_surface)
                        .border_1()
                        .border_color(glass_hairline)
                        .child(
                            text("Verify Offline Files")
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || theme().foreground),
                        ),
                ),
        )
}
