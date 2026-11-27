//! Typed [`EngineCommand`] dispatch handlers executed on [`AppEngine`]'s Tokio runtime.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use resonance_contract::{
    AppEvent, AuthSession, CacheStore, CatalogExtProvider, LIBRARY_SNAPSHOT_METADATA_KEY,
    LibrarySnapshot, MusicProvider, MutationCommand,
};
use resonance_core::innertube::auth::redact_secrets;
use resonance_core::playback::PlaybackController;

use super::{AppEngine, EngineCommand, LiveUpdate};
use crate::platform::platform_auth_status;

impl AppEngine {
    /// Dispatches a typed [`EngineCommand`] onto the background Tokio runtime.
    pub fn dispatch(&self, cmd: EngineCommand) {
        match cmd {
            // Catalog & Search
            EngineCommand::LoadHome
            | EngineCommand::LoadExplore
            | EngineCommand::LoadMoodPlaylists { .. }
            | EngineCommand::Search { .. }
            | EngineCommand::SearchSuggestions { .. }
            | EngineCommand::SearchContinuation { .. }
            | EngineCommand::BrowseCollection { .. } => self.dispatch_catalog(cmd),

            // Playback & Queue
            EngineCommand::PlayTrack(_)
            | EngineCommand::PlayQueue { .. }
            | EngineCommand::PlayNext(_)
            | EngineCommand::PlayLast(_)
            | EngineCommand::NextTrack
            | EngineCommand::PreviousTrack
            | EngineCommand::JumpQueue(_)
            | EngineCommand::MoveQueueItem { .. }
            | EngineCommand::RemoveQueueItem(_)
            | EngineCommand::ClearQueue
            | EngineCommand::TogglePlayPause
            | EngineCommand::SeekMs(_)
            | EngineCommand::SetVolume(_)
            | EngineCommand::SetShuffle(_)
            | EngineCommand::SetRepeatMode(_)
            | EngineCommand::SetAutoplayRadio(_)
            | EngineCommand::SetSponsorBlock(_)
            | EngineCommand::SetSleepTimer(_)
            | EngineCommand::SetPlaybackSpeed { .. } => self.dispatch_playback(cmd),

            // Auth, Config, Cache, Settings & Optimistic Mutations
            EngineCommand::SignIn(_)
            | EngineCommand::SignOut
            | EngineCommand::RefreshRemoteConfig
            | EngineCommand::ClearCaches
            | EngineCommand::PersistSettings { .. }
            | EngineCommand::SetAtrSync(_)
            | EngineCommand::SetLike { .. }
            | EngineCommand::EditPlaylist { .. }
            | EngineCommand::ReportAtr(_) => self.dispatch_account_and_system(cmd),

            // Offline Vault
            EngineCommand::EnqueueDownload(_)
            | EngineCommand::PauseDownload(_)
            | EngineCommand::ResumeDownload(_)
            | EngineCommand::CancelDownload(_)
            | EngineCommand::DeleteDownload(_)
            | EngineCommand::PinCollection { .. }
            | EngineCommand::UnpinCollection(_) => self.dispatch_vault(cmd),
        }
    }

    fn dispatch_catalog(&self, cmd: EngineCommand) {
        let provider = self.provider.clone();
        let tx = self.tx.clone();
        match cmd {
            EngineCommand::LoadHome => {
                self.rt.spawn(async move {
                    let res = provider.home().await.map_err(|e| e.to_string());
                    let _ = tx.send(LiveUpdate::HomeLoaded(res));
                });
            }
            EngineCommand::LoadExplore => {
                self.rt.spawn(async move {
                    let res = CatalogExtProvider::explore(provider.as_ref())
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(LiveUpdate::ExplorePageLoaded(res));
                });
            }
            EngineCommand::LoadMoodPlaylists {
                generation,
                mood,
                params,
            } => {
                self.rt.spawn(async move {
                    let res = provider
                        .mood_playlists(&params)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(LiveUpdate::MoodPlaylistsLoaded {
                        generation,
                        mood,
                        result: res,
                    });
                });
            }
            EngineCommand::Search {
                generation,
                query,
                params,
            } => {
                self.rt.spawn(async move {
                    match provider.search_with_params_and_plays(&query, params).await {
                        Ok((res, plays)) => {
                            let _ = tx.send(LiveUpdate::SearchLoaded {
                                generation,
                                query,
                                result: Ok(res),
                                plays,
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(LiveUpdate::SearchLoaded {
                                generation,
                                query,
                                result: Err(e.to_string()),
                                plays: HashMap::new(),
                            });
                        }
                    }
                });
            }
            EngineCommand::SearchSuggestions { prefix } => {
                if prefix.trim().is_empty() {
                    return;
                }
                self.rt.spawn(async move {
                    if let Ok(suggestions) = provider.search_suggestions(&prefix).await {
                        let _ = tx.send(LiveUpdate::SearchSuggestionsLoaded {
                            prefix,
                            suggestions,
                        });
                    }
                });
            }
            EngineCommand::SearchContinuation {
                generation,
                query,
                category,
                continuation,
            } => {
                self.rt.spawn(async move {
                    let res = provider
                        .search_categorized(&query, category, Some(&continuation))
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(LiveUpdate::SearchContinuationLoaded {
                        generation,
                        query,
                        result: res,
                    });
                });
            }
            EngineCommand::BrowseCollection {
                generation,
                browse_id,
            } => {
                self.rt.spawn(async move {
                    let res = provider
                        .browse_collection(&browse_id)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(LiveUpdate::CollectionLoaded {
                        generation,
                        result: res,
                    });
                });
            }
            _ => unreachable!("non-catalog command routed to dispatch_catalog"),
        }
    }

    fn dispatch_playback(&self, cmd: EngineCommand) {
        let controller = self.controller.clone();
        let tx = self.tx.clone();
        match cmd {
            EngineCommand::PlayTrack(track) => {
                let provider = self.provider.clone();
                self.rt.spawn(async move {
                    let seed_id = track.id.clone();
                    if let Err(e) = controller.play_track(track).await {
                        let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                            "Playback failed: {e}"
                        ))));
                    }
                    let _ = tx.send(LiveUpdate::PoTokenDiagnostics(
                        provider.po_token_diagnostics(),
                    ));
                    let _ = controller.expand_autoplay_queue(&seed_id).await;
                });
            }
            EngineCommand::PlayQueue {
                tracks,
                start_index,
                shuffle,
            } => {
                let provider = self.provider.clone();
                self.rt.spawn(async move {
                    if let Err(e) = controller.play_tracks(tracks, start_index).await {
                        let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                            "Playback failed: {e}"
                        ))));
                        return;
                    }
                    if shuffle {
                        controller.set_shuffle(true).await;
                    }
                    let _ = tx.send(LiveUpdate::PoTokenDiagnostics(
                        provider.po_token_diagnostics(),
                    ));
                    expand_autoplay_for_current(&controller).await;
                });
            }
            EngineCommand::PlayNext(track) => {
                self.rt.spawn(async move {
                    let _ = controller.play_next(track).await;
                });
            }
            EngineCommand::PlayLast(track) => {
                self.rt.spawn(async move {
                    let _ = controller.play_last(track).await;
                });
            }
            EngineCommand::NextTrack => {
                self.rt.spawn(async move {
                    match controller.next().await {
                        Ok(()) => expand_autoplay_for_current(&controller).await,
                        Err(e) => {
                            let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                                "Next track failed: {e}"
                            ))));
                        }
                    }
                });
            }
            EngineCommand::PreviousTrack => {
                self.rt.spawn(async move {
                    if let Err(e) = controller.previous().await {
                        let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                            "Previous track failed: {e}"
                        ))));
                    }
                });
            }
            EngineCommand::JumpQueue(index) => {
                self.rt.spawn(async move {
                    match controller.jump_to_queue_index(index).await {
                        Ok(()) => expand_autoplay_for_current(&controller).await,
                        Err(e) => {
                            let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                                "Queue jump failed: {e}"
                            ))));
                        }
                    }
                });
            }
            EngineCommand::MoveQueueItem { from, to } => {
                self.rt.spawn(async move {
                    let _ = controller.reorder_queue(from, to).await;
                });
            }
            EngineCommand::RemoveQueueItem(index) => {
                self.rt.spawn(async move {
                    let _ = controller.remove_from_queue(index).await;
                });
            }
            EngineCommand::ClearQueue => {
                self.rt.spawn(async move {
                    let _ = controller.clear_queue().await;
                });
            }
            EngineCommand::TogglePlayPause => {
                self.rt.spawn(async move {
                    if let Err(e) = controller.toggle_play_pause().await {
                        let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                            "Playback failed: {e}"
                        ))));
                    }
                });
            }
            EngineCommand::SeekMs(position_ms) => {
                self.rt.spawn(async move {
                    if let Err(e) = controller.seek_ms(position_ms).await {
                        let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                            "Seek failed: {e}"
                        ))));
                    }
                });
            }
            EngineCommand::SetVolume(volume) => {
                self.rt.spawn(async move {
                    let _ = controller.set_volume(volume.clamp(0.0, 1.0)).await;
                });
            }
            EngineCommand::SetShuffle(enabled) => {
                self.rt.spawn(async move {
                    controller.set_shuffle(enabled).await;
                });
            }
            EngineCommand::SetRepeatMode(mode) => {
                self.rt.spawn(async move {
                    controller.set_repeat_mode(mode).await;
                });
            }
            EngineCommand::SetAutoplayRadio(enabled) => {
                self.rt.spawn(async move {
                    controller.set_autoplay_radio(enabled).await;
                });
            }
            EngineCommand::SetSponsorBlock(enabled) => {
                self.rt.spawn(async move {
                    controller.set_sponsorblock_enabled(enabled).await;
                });
            }
            EngineCommand::SetSleepTimer(state) => {
                self.rt.spawn(async move {
                    controller.set_sleep_timer(state).await;
                });
            }
            EngineCommand::SetPlaybackSpeed {
                speed,
                preserve_pitch,
            } => {
                let clamped = speed.clamp(0.25, 3.0);
                #[cfg(not(target_os = "android"))]
                {
                    let _ = preserve_pitch;
                    crate::platform::nsh_audio::nsh_audio_bus().send_control(
                        crate::platform::nsh_audio::NshAudioControl::SetPlaybackSpeed(clamped),
                    );
                }
                #[cfg(target_os = "android")]
                {
                    let bridge = self.android_bridge.clone();
                    self.rt.spawn(async move {
                        let _ = bridge.set_playback_speed(clamped, preserve_pitch).await;
                    });
                }
            }
            _ => unreachable!("non-playback command routed to dispatch_playback"),
        }
    }

    fn dispatch_account_and_system(&self, cmd: EngineCommand) {
        match cmd {
            EngineCommand::SignIn(cookie_header) => {
                let provider = self.provider.clone();
                let tx = self.tx.clone();
                let bus = self.bus.clone();
                let auth_gen = self.auth_generation.clone();
                let my_gen = auth_gen.fetch_add(1, Ordering::SeqCst) + 1;
                self.rt.spawn(async move {
                    let session = AuthSession {
                        cookie_header,
                        auth_user: 0,
                        page_id: None,
                        visitor_data: provider.current_visitor_data(),
                        account_name: None,
                        is_premium: false,
                    };
                    match provider.sync_session(Some(&session)).await {
                        Ok(()) => {
                            if auth_gen.load(Ordering::SeqCst) != my_gen {
                                return;
                            }
                            let mut probed_name = None;
                            if let Ok(Some(profile)) = provider.probe_account_profile().await {
                                if auth_gen.load(Ordering::SeqCst) != my_gen {
                                    return;
                                }
                                if !profile.account_name.is_empty() {
                                    probed_name = Some(profile.account_name.clone());
                                }
                                let _ = tx.send(LiveUpdate::AccountProfileUpdated(Some(profile)));
                            }
                            if auth_gen.load(Ordering::SeqCst) != my_gen {
                                return;
                            }
                            bus.publish(AppEvent::AuthStateChanged {
                                signed_in: true,
                                account_name: probed_name,
                            });
                            let status = platform_auth_status(provider.current_session().as_ref());
                            let _ = tx.send(LiveUpdate::AuthStatus(status));
                            let _ = tx.send(LiveUpdate::AuthUpdated {
                                signed_in: true,
                                banner: "Signed in securely (stored in OS Keystore)".into(),
                            });
                            let snap = provider.refresh_and_cache_library_snapshot().await;
                            if auth_gen.load(Ordering::SeqCst) == my_gen {
                                let _ = tx.send(LiveUpdate::LibraryLoaded(snap));
                            }
                        }
                        Err(e) => {
                            if auth_gen.load(Ordering::SeqCst) == my_gen {
                                let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                                    "Sign-in failed: {e}"
                                ))));
                            }
                        }
                    }
                });
            }
            EngineCommand::SignOut => {
                let provider = self.provider.clone();
                let cache = self.cache.clone();
                let tx = self.tx.clone();
                let bus = self.bus.clone();
                let auth_gen = self.auth_generation.clone();
                let explicit_sign_out_pending = self.explicit_sign_out_pending.clone();
                let my_gen = auth_gen.fetch_add(1, Ordering::SeqCst) + 1;
                self.rt.spawn(async move {
                    let _ = provider.sync_session(None).await;
                    if auth_gen.load(Ordering::SeqCst) != my_gen {
                        return;
                    }
                    let _ = cache.evict_metadata(LIBRARY_SNAPSHOT_METADATA_KEY);
                    explicit_sign_out_pending.fetch_add(1, Ordering::SeqCst);
                    bus.publish(AppEvent::AuthStateChanged {
                        signed_in: false,
                        account_name: None,
                    });
                    let status = platform_auth_status(provider.current_session().as_ref());
                    let _ = tx.send(LiveUpdate::AccountProfileUpdated(None));
                    let _ = tx.send(LiveUpdate::AuthStatus(status));
                    let _ = tx.send(LiveUpdate::AuthUpdated {
                        signed_in: false,
                        banner: "Signed out — running in anonymous mode".into(),
                    });
                    let _ = tx.send(LiveUpdate::LibraryLoaded(LibrarySnapshot::default()));
                });
            }
            EngineCommand::RefreshRemoteConfig => {
                let rc = self.remote_config.clone();
                let provider = self.provider.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    if let Ok(Some(matrix)) = rc.sync_client_matrix(true).await {
                        provider.update_client_matrix(matrix);
                    }
                    let _ = rc.sync_solver_manifest(true).await;
                    let _ = tx.send(LiveUpdate::RemoteConfigUpdated(rc.status()));
                });
            }
            EngineCommand::ClearCaches => {
                let cache = self.cache.clone();
                let chunks = self.chunks.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let _ = chunks.clear_all();
                    let _ = cache.clear_all_caches();
                    let stats = cache.stats().unwrap_or_default();
                    let _ = tx.send(LiveUpdate::CacheStatsUpdated(stats));
                });
            }
            EngineCommand::PersistSettings {
                settings,
                immediate,
            } => {
                let _ = self.settings_tx.send((settings, immediate));
            }
            EngineCommand::SetAtrSync(enabled) => {
                self.atr_enabled.store(enabled, Ordering::SeqCst);
            }
            EngineCommand::SetLike { video_id, action } => {
                self.enqueue_mutation(MutationCommand::SetLike { video_id, action });
            }
            EngineCommand::EditPlaylist { playlist_id, op } => {
                self.enqueue_mutation(MutationCommand::EditPlaylist { playlist_id, op });
            }
            EngineCommand::ReportAtr(report) => {
                self.enqueue_mutation(MutationCommand::ReportAtr(report));
            }
            _ => unreachable!("non-account command routed to dispatch_account_and_system"),
        }
    }

    fn dispatch_vault(&self, cmd: EngineCommand) {
        let vault = self.vault.clone();
        let tx = self.tx.clone();
        match cmd {
            EngineCommand::EnqueueDownload(track) => {
                self.rt.spawn(async move {
                    if let Err(e) = vault.enqueue_track(track).await {
                        let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                            "Download failed: {e}"
                        ))));
                    }
                });
            }
            EngineCommand::PauseDownload(video_id) => {
                self.rt.spawn(async move {
                    let _ = vault.pause_track(&video_id).await;
                });
            }
            EngineCommand::ResumeDownload(video_id) => {
                self.rt.spawn(async move {
                    if let Err(e) = vault.resume_track(&video_id).await {
                        let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                            "Resume failed: {e}"
                        ))));
                    }
                });
            }
            EngineCommand::CancelDownload(video_id) => {
                self.rt.spawn(async move {
                    let _ = vault.cancel_track(&video_id).await;
                });
            }
            EngineCommand::DeleteDownload(video_id) => {
                self.rt.spawn(async move {
                    let _ = vault.delete_track(&video_id).await;
                });
            }
            EngineCommand::PinCollection {
                id,
                kind,
                title,
                subtitle,
                artwork_url,
                tracks,
            } => {
                self.rt.spawn(async move {
                    if let Err(e) = vault
                        .pin_collection(
                            &id,
                            kind,
                            &title,
                            subtitle.as_deref(),
                            artwork_url.as_deref(),
                            &tracks,
                        )
                        .await
                    {
                        let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                            "Pin failed: {e}"
                        ))));
                    } else if let Ok(list) = vault.list_pinned_collections() {
                        let _ = tx.send(LiveUpdate::PinnedCollectionsLoaded(list));
                    }
                });
            }
            EngineCommand::UnpinCollection(collection_id) => {
                self.rt.spawn(async move {
                    let _ = vault.unpin_collection(&collection_id);
                    if let Ok(list) = vault.list_pinned_collections() {
                        let _ = tx.send(LiveUpdate::PinnedCollectionsLoaded(list));
                    }
                });
            }
            _ => unreachable!("non-vault command routed to dispatch_vault"),
        }
    }

    fn enqueue_mutation(&self, cmd: MutationCommand) {
        if let Err(e) = self.mutations.enqueue(cmd.clone()) {
            let _ = self.tx.send(LiveUpdate::MutationFailed {
                command: cmd,
                reason: e.to_string(),
            });
        }
    }
}

async fn expand_autoplay_for_current(controller: &Arc<PlaybackController>) {
    if let Some(track) = controller.snapshot().await.current_track {
        let _ = controller.expand_autoplay_queue(&track.id).await;
    }
}
