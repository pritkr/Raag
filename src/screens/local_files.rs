//! H3.2 Local Library & Folder Manager screen (`src/screens/local_files.rs`).
//!
//! Zero raw color literals, `>= 48dp` mobile touch targets, `.pb_safe_area()`,
//! and no blur inside `virtual_list` cells.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use nsh_base::{button, input, theme};
use nsh_core::file_dialog::OpenDialog;
use nsh_core::prelude::*;
use resonance_contract::{LocalFolderRoot, LocalScanProgress, LocalTrackRecord, Track};

use crate::store::LocalLibrarySubTab;
use crate::ui::controls::{
    artwork_box, glass_pill_button, hero_pill_button, secondary_button, track_row_view,
};
use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_status_ok, glass_hairline, pill_glass_surface, radius_card,
};

/// View-model props & callbacks for `local_files_screen`.
#[derive(Clone)]
pub struct LocalFilesProps {
    pub roots: Signal<Vec<LocalFolderRoot>>,
    pub tracks: Signal<Vec<LocalTrackRecord>>,
    pub progress: Signal<LocalScanProgress>,
    pub reduced_motion: Signal<bool>,
    pub on_add_folder: Rc<dyn Fn(PathBuf)>,
    pub on_remove_folder: Rc<dyn Fn(String)>,
    pub on_rescan: Rc<dyn Fn()>,
    pub on_play_track: Rc<dyn Fn(Track, Vec<Track>)>,
}

/// Renders the Local Library & Folder Manager view.
pub fn local_files_screen(props: LocalFilesProps) -> impl IntoElement {
    let active_tab = signal(LocalLibrarySubTab::Tracks);
    let manual_path = signal(String::new());
    let filter_query = signal(String::new());
    let picker_status = signal(Option::<String>::None);

    let on_add_for_picker = props.on_add_folder.clone();
    let on_add_for_manual = props.on_add_folder.clone();
    let on_rescan = props.on_rescan.clone();

    view()
        .test_id("local-files-screen")
        .flex_1()
        .flex_col()
        .gap_4()
        .p_4()
        .pb_safe_area()
        .child(
            // Folder Manager header card
            view()
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
                        .gap_2()
                        .flex_wrap()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(icon(IconKind::Library, 18.0, move || theme().foreground))
                                .child(
                                    text("Local Music Folders")
                                        .text_base()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().foreground),
                                ),
                        )
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(
                                    hero_pill_button("Add Music Folder", None, {
                                        move || {
                                            let add_cb = on_add_for_picker.clone();
                                            spawn(async move {
                                                match pick_paths(OpenDialog::folders()).await {
                                                    Ok(Some(paths)) => {
                                                        for path in paths {
                                                            add_cb(path);
                                                        }
                                                    }
                                                    Ok(None) => {}
                                                    Err(err) => {
                                                        picker_status.set(Some(format!(
                                                            "Folder picker unavailable ({err}); enter path below."
                                                        )));
                                                    }
                                                }
                                            });
                                        }
                                    }),
                                )
                                .child(secondary_button("Rescan", move || {
                                    on_rescan();
                                })),
                        ),
                )
                .child(
                    // Headless / fallback path input row
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            input()
                                .test_id("local-folder-path-input")
                                .flex_1()
                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                .placeholder("Enter folder path (e.g. /Users/you/Music)")
                                .value(move || manual_path.get())
                                .on_change(move |val: &str| manual_path.set(val.to_owned())),
                        )
                        .child(glass_pill_button("Add Path", None, move || {
                            let raw = manual_path.get();
                            let trimmed = raw.trim();
                            if !trimmed.is_empty() {
                                on_add_for_manual(PathBuf::from(trimmed));
                                manual_path.set(String::new());
                            }
                        })),
                )
                .child(when(
                    move || picker_status.get().is_some(),
                    move || {
                        let msg = picker_status.get().unwrap_or_default();
                        text(msg)
                            .text_xs()
                            .text_color(move || theme().muted_foreground)
                    },
                ))
                .child(when(
                    move || props.progress.get().scanning,
                    move || {
                        let p = props.progress.get();
                        view()
                            .px_3()
                            .py_2()
                            .rounded(radius_card)
                            .bg(pill_glass_surface)
                            .child(
                                text(format!(
                                    "Scanning… {} seen • {} indexed • {} unchanged",
                                    p.files_seen, p.files_indexed, p.files_skipped_unchanged
                                ))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(color_status_ok),
                            )
                    },
                ))
                .child(
                    // Registered roots list
                    dynamic(move || {
                        let roots = props.roots.get();
                        let on_remove = props.on_remove_folder.clone();
                        view()
                            .flex_col()
                            .gap_2()
                            .children(roots.into_iter().map(move |root| {
                                let remove_cb = on_remove.clone();
                                let path_for_btn = root.path.clone();
                                view()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .px_3()
                                    .py_2()
                                    .rounded(radius_card)
                                    .bg(move || theme().muted)
                                    .child(
                                        view()
                                            .flex_1()
                                            .flex_col()
                                            .child(
                                                text(root.path.clone())
                                                    .text_sm()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(move || theme().foreground),
                                            )
                                            .child(
                                                text(format!("{} tracks", root.track_count))
                                                    .text_xs()
                                                    .text_color(move || theme().muted_foreground),
                                            ),
                                    )
                                    .child(
                                        button()
                                            .test_id("local-remove-root-btn")
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                            .min_w(px(MOBILE_MIN_TOUCH_TARGET))
                                            .px_3()
                                            .rounded(radius_card)
                                            .accessibility_label("Remove Folder")
                                            .on_press(move || remove_cb(path_for_btn.clone()))
                                            .child(
                                                text("Remove")
                                                    .text_xs()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(move || theme().destructive),
                                            ),
                                    )
                            }))
                            .into_element()
                    }),
                ),
        )
        .child(
            // Sub-tabs: Tracks | Albums | Artists + inline filter & Play All
            {
                let on_play_track_summary = props.on_play_track.clone();
                dynamic(move || {
                    let current = active_tab.get();
                    let track_count = props.tracks.with(Vec::len);
                    let root_count = props.roots.with(Vec::len);
                    let on_play_all = on_play_track_summary.clone();
                let all_for_play = props.tracks.get();
                view()
                    .flex_col()
                    .gap_2()
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .children(LocalLibrarySubTab::all().iter().map(|&tab| {
                                        let is_active = tab == current;
                                        button()
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                            .px_4()
                                            .rounded_full()
                                            .bg(move || {
                                                if is_active {
                                                    theme().primary
                                                } else {
                                                    pill_glass_surface()
                                                }
                                            })
                                            .on_press(move || active_tab.set(tab))
                                            .child(
                                                text(tab.label())
                                                    .text_sm()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(move || {
                                                        if is_active {
                                                            theme().primary_foreground
                                                        } else {
                                                            theme().foreground
                                                        }
                                                    }),
                                            )
                                    })),
                            )
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        text(format!("{track_count} tracks · {root_count} folders"))
                                            .text_xs()
                                            .text_color(move || theme().muted_foreground),
                                    )
                                    .when(!all_for_play.is_empty(), |v| {
                                        let tracks_vec: Vec<Track> =
                                            all_for_play.iter().map(|r| r.track.clone()).collect();
                                        v.child(secondary_button("Play All", move || {
                                            if let Some(first) = tracks_vec.first().cloned() {
                                                on_play_all(first, tracks_vec.clone());
                                            }
                                        }))
                                    }),
                            ),
                    )
                    .child(
                        input()
                            .test_id("local-files-filter-input")
                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                            .placeholder("Filter local tracks...")
                            .value(move || filter_query.get())
                            .on_change(move |val: &str| filter_query.set(val.to_owned())),
                    )
                    .into_element()
                })
            }
        )
        .child(
            // Content list (zero blur inside virtual/list cells)
            dynamic(move || {
                let records = props.tracks.get();
                let q = filter_query.get().trim().to_lowercase();
                let all_tracks: Vec<Track> = records
                    .iter()
                    .map(|r| r.track.clone())
                    .filter(|t| {
                        q.is_empty()
                            || t.title.to_lowercase().contains(&q)
                            || t.artist_display().to_lowercase().contains(&q)
                            || t.album
                                .as_ref()
                                .is_some_and(|a| a.title.to_lowercase().contains(&q))
                    })
                    .collect();
                if all_tracks.is_empty() {
                    return view()
                        .flex_1()
                        .p_6()
                        .items_center()
                        .justify_center()
                        .child(
                            text("No local tracks indexed yet. Add a music folder above.")
                                .text_sm()
                                .text_color(move || theme().muted_foreground),
                        )
                        .into_element();
                }

                match active_tab.get() {
                    LocalLibrarySubTab::Tracks => {
                        let on_play = props.on_play_track.clone();
                        view()
                            .flex_1()
                            .flex_col()
                            .gap_2()
                            .children(all_tracks.iter().map(|track| {
                                let t = track.clone();
                                let queue = all_tracks.clone();
                                let play_cb = on_play.clone();
                                track_row_view(
                                    t.clone(),
                                    false,
                                    move || play_cb(t.clone(), queue.clone()),
                                    || {},
                                )
                            }))
                            .into_element()
                    }
                    LocalLibrarySubTab::Albums => {
                        let mut albums: BTreeMap<String, Vec<Track>> = BTreeMap::new();
                        for t in &all_tracks {
                            let key = t
                                .album
                                .as_ref()
                                .map(|a| a.title.clone())
                                .unwrap_or_else(|| "Unknown Album".to_owned());
                            albums.entry(key).or_default().push(t.clone());
                        }
                        let on_play = props.on_play_track.clone();
                        view()
                            .flex_1()
                            .flex_col()
                            .gap_2()
                            .children(albums.into_iter().map(|(title, tracks)| {
                                let art = tracks.first().and_then(|t| t.artwork_url.clone());
                                let mono = tracks
                                    .first()
                                    .map(Track::fallback_monogram)
                                    .unwrap_or_else(|| "AL".to_owned());
                                let first = tracks.first().cloned();
                                let queue = tracks.clone();
                                let play_cb = on_play.clone();
                                let count = tracks.len();
                                button()
                                    .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                    .p_2()
                                    .rounded(radius_card)
                                    .bg(move || theme().card)
                                    .on_press(move || {
                                        if let Some(t) = first.clone() {
                                            play_cb(t, queue.clone());
                                        }
                                    })
                                    .child(
                                        view()
                                            .flex_row()
                                            .items_center()
                                            .gap_3()
                                            .child(artwork_box(art, mono, 44.0, 8.0))
                                            .child(
                                                view()
                                                    .flex_col()
                                                    .child(
                                                        text(title.clone())
                                                            .text_sm()
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .text_color(move || theme().foreground),
                                                    )
                                                    .child(
                                                        text(format!("{count} tracks"))
                                                            .text_xs()
                                                            .text_color(move || {
                                                                theme().muted_foreground
                                                            }),
                                                    ),
                                            ),
                                    )
                            }))
                            .into_element()
                    }
                    LocalLibrarySubTab::Artists => {
                        let mut artists: BTreeMap<String, Vec<Track>> = BTreeMap::new();
                        for t in &all_tracks {
                            artists
                                .entry(t.artist_display())
                                .or_default()
                                .push(t.clone());
                        }
                        let on_play = props.on_play_track.clone();
                        view()
                            .flex_1()
                            .flex_col()
                            .gap_2()
                            .children(artists.into_iter().map(|(name, tracks)| {
                                let first = tracks.first().cloned();
                                let queue = tracks.clone();
                                let play_cb = on_play.clone();
                                let count = tracks.len();
                                button()
                                    .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                    .p_3()
                                    .rounded(radius_card)
                                    .bg(move || theme().card)
                                    .on_press(move || {
                                        if let Some(t) = first.clone() {
                                            play_cb(t, queue.clone());
                                        }
                                    })
                                    .child(
                                        view()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                text(name.clone())
                                                    .text_sm()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(move || theme().foreground),
                                            )
                                            .child(
                                                text(format!("{count} tracks"))
                                                    .text_xs()
                                                    .text_color(move || theme().muted_foreground),
                                            ),
                                    )
                            }))
                            .into_element()
                    }
                }
            }),
        )
}
