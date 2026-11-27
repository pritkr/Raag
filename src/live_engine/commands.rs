//! Typed [`EngineCommand`] dispatch handlers executed on [`AppEngine`]'s Tokio runtime.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use resonance_contract::{
    AppEvent, ArtistRef, AuthSession, AuthoritativePlaybackState, CacheStore, CastSessionState,
    CatalogExtProvider, LIBRARY_SNAPSHOT_METADATA_KEY, LibrarySnapshot, MusicProvider,
    MutationCommand, NowPlayingMetadata, PlaylistEditOp, RoomRole, RoomTrackRef, RoomWireMessage,
    Track, UpdatePhase,
};
use resonance_core::auth_flows::{TvOAuthClient, TvOAuthConfig, redact_oauth_secrets};
use resonance_core::casting::CastDiscoveryService;
use resonance_core::innertube::auth::redact_secrets;
use resonance_core::local_media::{scan_all_roots, scan_root};
use resonance_core::playback::PlaybackController;
use resonance_core::providers::HybridMusicProvider;
use resonance_core::rooms::RoomController;
use resonance_core::rooms::drift::ThreeZoneDriftAction;
use resonance_core::unix_secs;

use super::{AppEngine, EngineCommand, LiveUpdate, RoomTaskCmd};
use crate::platform::platform_auth_status;

impl AppEngine {
    /// Dispatches a typed [`EngineCommand`] onto the background Tokio runtime.
    pub fn dispatch(&self, cmd: EngineCommand) {
        match cmd {
            // Catalog & Search
            EngineCommand::LoadHome
            | EngineCommand::LoadHomeWithMood { .. }
            | EngineCommand::LoadExplore
            | EngineCommand::LoadMoodPlaylists { .. }
            | EngineCommand::LoadAlbumEntity { .. }
            | EngineCommand::LoadArtistEntity { .. }
            | EngineCommand::LoadRelatedTracks { .. }
            | EngineCommand::Search { .. }
            | EngineCommand::SearchSuggestions { .. }
            | EngineCommand::SearchContinuation { .. }
            | EngineCommand::BrowseCollection { .. } => self.dispatch_catalog(cmd),

            // Playback, Queue & DSP
            EngineCommand::PlayTrack(_)
            | EngineCommand::PlayQueue { .. }
            | EngineCommand::PlayNext(_)
            | EngineCommand::PlayLast(_)
            | EngineCommand::NextTrack
            | EngineCommand::PreviousTrack
            | EngineCommand::JumpQueue(_)
            | EngineCommand::MoveQueueItem { .. }
            | EngineCommand::RemoveQueueItem(_)
            | EngineCommand::ClearUpNext
            | EngineCommand::ClearQueue
            | EngineCommand::TogglePlayPause
            | EngineCommand::SeekMs(_)
            | EngineCommand::SetVolume(_)
            | EngineCommand::SetShuffle(_)
            | EngineCommand::SetRepeatMode(_)
            | EngineCommand::SetAutoplayRadio(_)
            | EngineCommand::SetSponsorBlock(_)
            | EngineCommand::SetSleepTimer(_)
            | EngineCommand::SetPlaybackSpeed { .. }
            | EngineCommand::UpdateDsp(_) => self.dispatch_playback(cmd),

            // Auth, Config, Cache, Settings & Optimistic Mutations
            EngineCommand::SignIn(_)
            | EngineCommand::SignOut
            | EngineCommand::StartTvOAuth
            | EngineCommand::CancelTvOAuth
            | EngineCommand::LaunchAndroidWebViewSignIn
            | EngineCommand::RefreshRemoteConfig
            | EngineCommand::ClearCaches
            | EngineCommand::PersistSettings { .. }
            | EngineCommand::SetAtrSync(_)
            | EngineCommand::SetLike { .. }
            | EngineCommand::EditPlaylist { .. }
            | EngineCommand::CreatePlaylist { .. }
            | EngineCommand::RenamePlaylist { .. }
            | EngineCommand::DeletePlaylist { .. }
            | EngineCommand::ReportAtr(_) => self.dispatch_account_and_system(cmd),

            // Offline Vault
            EngineCommand::EnqueueDownload(_)
            | EngineCommand::PauseDownload(_)
            | EngineCommand::ResumeDownload(_)
            | EngineCommand::CancelDownload(_)
            | EngineCommand::DeleteDownload(_)
            | EngineCommand::PinCollection { .. }
            | EngineCommand::UnpinCollection(_) => self.dispatch_vault(cmd),

            // Local Media, Listen Together, LAN Cast & Signed Auto-Updater
            EngineCommand::LoadLocalMedia
            | EngineCommand::AddLocalFolder(_)
            | EngineCommand::RemoveLocalFolder(_)
            | EngineCommand::RescanLocalMedia => self.dispatch_local_media(cmd),

            EngineCommand::CreateRoom { .. }
            | EngineCommand::JoinRoom { .. }
            | EngineCommand::LeaveRoom
            | EngineCommand::VoteSkipRoom
            | EngineCommand::SendRoomReaction(_)
            | EngineCommand::KickRoomParticipant(_)
            | EngineCommand::TransferRoomHost(_) => self.dispatch_rooms(cmd),

            EngineCommand::StartCastDiscovery
            | EngineCommand::StopCastDiscovery
            | EngineCommand::ConnectCastDevice(_)
            | EngineCommand::DisconnectCast
            | EngineCommand::SetCastVolume(_) => self.dispatch_cast(cmd),

            EngineCommand::CheckForUpdates { .. }
            | EngineCommand::DownloadUpdate(_)
            | EngineCommand::CancelUpdateDownload
            | EngineCommand::InstallUpdateOnQuit
            | EngineCommand::RestartToInstallUpdate => self.dispatch_updater(cmd),
        }
    }

    fn dispatch_catalog(&self, cmd: EngineCommand) {
        let provider = self.provider.clone();
        let tx = self.tx.clone();
        match cmd {
            EngineCommand::LoadHome => {
                self.rt.spawn(async move {
                    match provider.home_with_mood(None).await {
                        Ok((sections, chips)) => {
                            let _ = tx.send(LiveUpdate::HomeMoodLoaded {
                                mood: None,
                                chips,
                                result: Ok(sections),
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(LiveUpdate::HomeMoodLoaded {
                                mood: None,
                                chips: Vec::new(),
                                result: Err(e.to_string()),
                            });
                        }
                    }
                });
            }
            EngineCommand::LoadHomeWithMood { mood, params } => {
                self.rt.spawn(async move {
                    match provider.home_with_mood(params.as_deref()).await {
                        Ok((sections, chips)) => {
                            let _ = tx.send(LiveUpdate::HomeMoodLoaded {
                                mood,
                                chips,
                                result: Ok(sections),
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(LiveUpdate::HomeMoodLoaded {
                                mood,
                                chips: Vec::new(),
                                result: Err(e.to_string()),
                            });
                        }
                    }
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
            EngineCommand::LoadAlbumEntity {
                generation,
                browse_id,
            } => {
                self.rt.spawn(async move {
                    let res = CatalogExtProvider::fetch_album(provider.as_ref(), &browse_id)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(LiveUpdate::AlbumEntityLoaded {
                        generation,
                        result: res,
                    });
                });
            }
            EngineCommand::LoadArtistEntity {
                generation,
                channel_id,
            } => {
                self.rt.spawn(async move {
                    let res = CatalogExtProvider::fetch_artist(provider.as_ref(), &channel_id)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(LiveUpdate::ArtistEntityLoaded {
                        generation,
                        result: res,
                    });
                });
            }
            EngineCommand::LoadRelatedTracks { video_id } => {
                self.rt.spawn(async move {
                    fetch_and_send_related(&provider, &tx, video_id).await;
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
                    fetch_and_send_related(&provider, &tx, seed_id).await;
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
                    expand_autoplay_for_current(&controller, &provider, &tx).await;
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
                let provider = self.provider.clone();
                self.rt.spawn(async move {
                    match controller.next().await {
                        Ok(()) => expand_autoplay_for_current(&controller, &provider, &tx).await,
                        Err(e) => {
                            let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                                "Next track failed: {e}"
                            ))));
                        }
                    }
                });
            }
            EngineCommand::PreviousTrack => {
                let provider = self.provider.clone();
                self.rt.spawn(async move {
                    match controller.previous().await {
                        Ok(()) => expand_autoplay_for_current(&controller, &provider, &tx).await,
                        Err(e) => {
                            let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                                "Previous track failed: {e}"
                            ))));
                        }
                    }
                });
            }
            EngineCommand::JumpQueue(index) => {
                let provider = self.provider.clone();
                self.rt.spawn(async move {
                    match controller.jump_to_queue_index(index).await {
                        Ok(()) => expand_autoplay_for_current(&controller, &provider, &tx).await,
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
            EngineCommand::ClearUpNext => {
                self.rt.spawn(async move {
                    let _ = controller.clear_up_next().await;
                });
            }
            EngineCommand::ClearQueue => {
                self.rt.spawn(async move {
                    let _ = controller.clear_queue().await;
                });
            }
            EngineCommand::TogglePlayPause => {
                let cast = self.cast_manager.clone();
                self.rt.spawn(async move {
                    if let CastSessionState::Connected { playing, .. } = cast.state().await {
                        let res = if playing {
                            cast.pause().await
                        } else {
                            cast.play().await
                        };
                        match res {
                            Ok(()) => {
                                let _ = tx.send(LiveUpdate::CastSessionUpdated(cast.state().await));
                            }
                            Err(e) => {
                                let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(
                                    &format!("Cast playback toggle failed: {e}"),
                                )));
                            }
                        }
                        return;
                    }
                    if let Err(e) = controller.toggle_play_pause().await {
                        let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                            "Playback failed: {e}"
                        ))));
                    }
                });
            }
            EngineCommand::SeekMs(position_ms) => {
                let cast = self.cast_manager.clone();
                self.rt.spawn(async move {
                    if matches!(cast.state().await, CastSessionState::Connected { .. }) {
                        if let Err(e) = cast.seek_ms(position_ms).await {
                            let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                                "Cast seek failed: {e}"
                            ))));
                        } else {
                            let _ = tx.send(LiveUpdate::CastSessionUpdated(cast.state().await));
                        }
                        return;
                    }
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
            EngineCommand::UpdateDsp(cfg) => {
                #[cfg(not(target_os = "android"))]
                {
                    crate::platform::nsh_audio::nsh_audio_bus()
                        .send_control(crate::platform::nsh_audio::NshAudioControl::UpdateDsp(cfg));
                }
                #[cfg(target_os = "android")]
                {
                    crate::platform::android::update_android_dsp(&cfg);
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
            EngineCommand::StartTvOAuth => {
                let tx = self.tx.clone();
                let cancel_slot = self.tv_oauth_cancel.clone();
                // Cancel any previously active device-code poll before starting a new one.
                let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
                if let Ok(mut guard) = cancel_slot.lock() {
                    if let Some(prev) = guard.replace(cancel_tx) {
                        let _ = prev.send(true);
                    }
                }
                self.rt.spawn(async move {
                    let client = match TvOAuthClient::new(TvOAuthConfig::from_env_or_default()) {
                        Ok(c) => c,
                        Err(err) => {
                            let _ = tx.send(LiveUpdate::TvOAuthCompleted {
                                ok: false,
                                message: format!(
                                    "TV OAuth client init failed: {}",
                                    redact_oauth_secrets(&err.to_string())
                                ),
                            });
                            return;
                        }
                    };
                    let _ = tx.send(LiveUpdate::TvOAuthStatus(
                        "Contacting Google OAuth device endpoint…".into(),
                    ));
                    match client.request_device_code().await {
                        Ok(dc) => {
                            let _ = tx.send(LiveUpdate::TvOAuthDeviceCode {
                                user_code: dc.user_code.clone(),
                                verification_url: dc.verification_url.clone(),
                                expires_in_secs: dc.expires_in,
                            });
                            let _ = tx.send(LiveUpdate::TvOAuthStatus(
                                "Waiting for approval at google.com/device…".into(),
                            ));
                            match client
                                .poll_for_token(&dc.device_code, dc.interval, dc.expires_in, cancel_rx)
                                .await
                            {
                                Ok(_tokens) => {
                                    // Never log or expose raw access/refresh tokens in UI banners.
                                    let _ = tx.send(LiveUpdate::TvOAuthCompleted {
                                        ok: true,
                                        message: "Google TV device authorization approved. For full YouTube Music library sync, also paste browser cookies if prompted.".into(),
                                    });
                                }
                                Err(err) => {
                                    let msg = redact_oauth_secrets(&err.to_string());
                                    let _ = tx.send(LiveUpdate::TvOAuthCompleted {
                                        ok: false,
                                        message: format!("TV OAuth ended: {msg}"),
                                    });
                                }
                            }
                        }
                        Err(err) => {
                            let _ = tx.send(LiveUpdate::TvOAuthCompleted {
                                ok: false,
                                message: format!(
                                    "Device code request failed: {}",
                                    redact_oauth_secrets(&err.to_string())
                                ),
                            });
                        }
                    }
                    if let Ok(mut guard) = cancel_slot.lock() {
                        guard.take();
                    }
                });
            }
            EngineCommand::CancelTvOAuth => {
                if let Ok(mut guard) = self.tv_oauth_cancel.lock()
                    && let Some(cancel_tx) = guard.take()
                {
                    let _ = cancel_tx.send(true);
                }
            }
            EngineCommand::LaunchAndroidWebViewSignIn => {
                #[cfg(target_os = "android")]
                {
                    self.android_bridge.launch_sign_in_webview();
                    let _ = self.tx.send(LiveUpdate::StatusBanner(
                        "Opened YouTube Music sign-in WebView".into(),
                    ));
                }
                #[cfg(not(target_os = "android"))]
                {
                    let _ = self.tx.send(LiveUpdate::StatusBanner(
                        "Embedded WebView sign-in is Android-only — use TV OAuth or Paste Cookies on desktop".into(),
                    ));
                }
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
                    let _ = provider.warm_player_solver().await;
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
            EngineCommand::CreatePlaylist {
                local_temp_id,
                title,
                description,
                privacy,
                initial_video_ids,
            } => {
                self.enqueue_mutation(MutationCommand::CreatePlaylist {
                    local_temp_id,
                    title,
                    description,
                    privacy,
                    initial_video_ids,
                });
            }
            EngineCommand::RenamePlaylist {
                playlist_id,
                new_title,
            } => {
                self.enqueue_mutation(MutationCommand::EditPlaylist {
                    playlist_id,
                    op: PlaylistEditOp::Rename {
                        title: new_title,
                        description: None,
                    },
                });
            }
            EngineCommand::DeletePlaylist { playlist_id } => {
                self.enqueue_mutation(MutationCommand::DeletePlaylist { playlist_id });
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

    fn dispatch_local_media(&self, cmd: EngineCommand) {
        let Some(store) = self.local_media.clone() else {
            let _ = self.tx.send(LiveUpdate::StatusBanner(
                "Local music database unavailable".into(),
            ));
            return;
        };
        let artwork = Some(self.artwork.clone());
        let proxy_base = self.proxy_base_url.clone();
        let tx = self.tx.clone();
        match cmd {
            EngineCommand::LoadLocalMedia => {
                self.rt.spawn(async move {
                    let roots = store.list_roots().unwrap_or_default();
                    let tracks = store.list_tracks().unwrap_or_default();
                    let _ = tx.send(LiveUpdate::LocalMediaLoaded { roots, tracks });
                });
            }
            EngineCommand::AddLocalFolder(path) => {
                self.rt.spawn(async move {
                    match store.add_root(&path, unix_secs()) {
                        Ok(added) => {
                            let tx_prog = tx.clone();
                            let cb = Arc::new(move |prog| {
                                let _ = tx_prog.send(LiveUpdate::LocalScanProgressUpdated(prog));
                            });
                            let root_pb = std::path::PathBuf::from(&added.path);
                            if let Err(e) =
                                scan_root(store.clone(), artwork, root_pb, proxy_base, Some(cb))
                                    .await
                            {
                                let _ = tx.send(LiveUpdate::StatusBanner(format!(
                                    "Local folder scan error: {e}"
                                )));
                            }
                            let roots = store.list_roots().unwrap_or_default();
                            let tracks = store.list_tracks().unwrap_or_default();
                            let count = tracks.len();
                            let _ = tx.send(LiveUpdate::LocalMediaLoaded { roots, tracks });
                            let _ = tx.send(LiveUpdate::StatusBanner(format!(
                                "Indexed local folder ({count} total local tracks)"
                            )));
                        }
                        Err(e) => {
                            let _ = tx.send(LiveUpdate::StatusBanner(format!(
                                "Couldn't add folder: {e}"
                            )));
                        }
                    }
                });
            }
            EngineCommand::RemoveLocalFolder(root_path) => {
                self.rt.spawn(async move {
                    let _ = store.remove_root(&root_path);
                    let roots = store.list_roots().unwrap_or_default();
                    let tracks = store.list_tracks().unwrap_or_default();
                    let _ = tx.send(LiveUpdate::LocalMediaLoaded { roots, tracks });
                    let _ = tx.send(LiveUpdate::StatusBanner(format!(
                        "Removed local folder: {root_path}"
                    )));
                });
            }
            EngineCommand::RescanLocalMedia => {
                self.rt.spawn(async move {
                    let tx_prog = tx.clone();
                    let cb = Arc::new(move |prog| {
                        let _ = tx_prog.send(LiveUpdate::LocalScanProgressUpdated(prog));
                    });
                    match scan_all_roots(store.clone(), artwork, proxy_base, Some(cb)).await {
                        Ok(prog) => {
                            let roots = store.list_roots().unwrap_or_default();
                            let tracks = store.list_tracks().unwrap_or_default();
                            let _ = tx.send(LiveUpdate::LocalMediaLoaded { roots, tracks });
                            let _ = tx.send(LiveUpdate::StatusBanner(format!(
                                "Scanned local library: {} indexed, {} unchanged",
                                prog.files_indexed, prog.files_skipped_unchanged
                            )));
                        }
                        Err(e) => {
                            let _ = tx.send(LiveUpdate::StatusBanner(format!(
                                "Local rescan failed: {e}"
                            )));
                        }
                    }
                });
            }
            _ => unreachable!("non-local-media command routed to dispatch_local_media"),
        }
    }

    fn dispatch_rooms(&self, cmd: EngineCommand) {
        match cmd {
            EngineCommand::CreateRoom {
                relay_url,
                display_name,
                queue_permission,
            } => {
                self.spawn_room_session(RoomSessionInit::Create {
                    relay_url,
                    display_name,
                    queue_permission,
                });
            }
            EngineCommand::JoinRoom {
                relay_url,
                room_code,
                display_name,
            } => {
                self.spawn_room_session(RoomSessionInit::Join {
                    relay_url,
                    room_code,
                    display_name,
                });
            }
            EngineCommand::LeaveRoom => {
                if let Ok(mut guard) = self.room_tx.lock()
                    && let Some(tx) = guard.take()
                {
                    let _ = tx.send(RoomTaskCmd::Leave);
                }
                let _ = self.tx.send(LiveUpdate::RoomSnapshotUpdated(None));
            }
            EngineCommand::VoteSkipRoom => {
                self.send_room_cmd(RoomTaskCmd::VoteSkip);
            }
            EngineCommand::SendRoomReaction(emoji) => {
                self.send_room_cmd(RoomTaskCmd::SendReaction(emoji));
            }
            EngineCommand::KickRoomParticipant(peer_id) => {
                self.send_room_cmd(RoomTaskCmd::KickParticipant(peer_id));
            }
            EngineCommand::TransferRoomHost(peer_id) => {
                self.send_room_cmd(RoomTaskCmd::TransferHost(peer_id));
            }
            _ => unreachable!("non-room command routed to dispatch_rooms"),
        }
    }

    fn send_room_cmd(&self, cmd: RoomTaskCmd) {
        if let Ok(guard) = self.room_tx.lock()
            && let Some(tx) = guard.as_ref()
        {
            let _ = tx.send(cmd);
        }
    }

    fn spawn_room_session(&self, init: RoomSessionInit) {
        if let Ok(mut guard) = self.room_tx.lock()
            && let Some(prev) = guard.take()
        {
            let _ = prev.send(RoomTaskCmd::Leave);
        }

        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<RoomTaskCmd>();
        if let Ok(mut guard) = self.room_tx.lock() {
            *guard = Some(cmd_tx);
        }

        let ui_tx = self.tx.clone();
        let controller = self.controller.clone();
        let bus = self.bus.clone();
        let room_tx_slot = self.room_tx.clone();

        self.rt.spawn(async move {
            let mut room = match init {
                RoomSessionInit::Create {
                    relay_url,
                    display_name,
                    queue_permission,
                } => match RoomController::create_room(&relay_url, &display_name, queue_permission).await {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = ui_tx.send(LiveUpdate::RoomSnapshotUpdated(None));
                        let _ = ui_tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                            "Room creation failed: {e}"
                        ))));
                        return;
                    }
                },
                RoomSessionInit::Join {
                    relay_url,
                    room_code,
                    display_name,
                } => match RoomController::join_room(&relay_url, &room_code, &display_name).await {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = ui_tx.send(LiveUpdate::RoomSnapshotUpdated(None));
                        let _ = ui_tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                            "Couldn't join room: {e}"
                        ))));
                        return;
                    }
                },
            };

            let snap = room.snapshot();
            let _ = ui_tx.send(LiveUpdate::StatusBanner(format!(
                "Connected to room {} ({})",
                snap.room_code,
                snap.transport.label()
            )));
            let _ = ui_tx.send(LiveUpdate::RoomSnapshotUpdated(Some(snap)));

            let mut bus_rx = bus.subscribe();
            let mut ping_interval = tokio::time::interval(Duration::from_secs(10));
            let mut host_rev: u64 = 1;
            let mut last_guest_track_id: Option<String> = None;

            loop {
                tokio::select! {
                    Some(rcmd) = cmd_rx.recv() => {
                        match rcmd {
                            RoomTaskCmd::Leave => {
                                let _ = room.leave().await;
                                let _ = ui_tx.send(LiveUpdate::RoomSnapshotUpdated(None));
                                break;
                            }
                            RoomTaskCmd::VoteSkip => {
                                match room.cast_vote_skip().await {
                                    Ok(resp) => {
                                        let _ = ui_tx.send(LiveUpdate::RoomVoteSkipUpdated {
                                            votes: resp.votes,
                                            threshold: resp.threshold,
                                            skipped: resp.skipped,
                                        });
                                        if resp.skipped && room.snapshot().role == RoomRole::Host {
                                            let _ = controller.next().await;
                                        }
                                    }
                                    Err(e) => {
                                        let _ = ui_tx.send(LiveUpdate::StatusBanner(format!("Vote skip failed: {e}")));
                                    }
                                }
                            }
                            RoomTaskCmd::SendReaction(emoji) => {
                                let _ = ui_tx.send(LiveUpdate::RoomReactionReceived(emoji));
                            }
                            RoomTaskCmd::KickParticipant(peer_id) => {
                                if let Err(e) = room.kick_participant(&peer_id).await {
                                    let _ = ui_tx.send(LiveUpdate::StatusBanner(format!("Kick failed: {e}")));
                                } else {
                                    let _ = ui_tx.send(LiveUpdate::RoomSnapshotUpdated(Some(room.snapshot())));
                                }
                            }
                            RoomTaskCmd::TransferHost(new_host_peer_id) => {
                                if let Err(e) = room.send_wire_message(RoomWireMessage::TransferHost { new_host_peer_id }).await {
                                    let _ = ui_tx.send(LiveUpdate::StatusBanner(format!("Transfer host failed: {e}")));
                                }
                            }
                        }
                    }
                    _ = ping_interval.tick() => {
                        let now_ms = unix_secs().saturating_mul(1000);
                        let _ = room.send_clock_ping(now_ms).await;
                    }
                    Ok(ev) = bus_rx.recv() => {
                        if let AppEvent::PlayerUpdated(player_snap) = ev
                            && room.snapshot().role == RoomRole::Host
                        {
                            host_rev = host_rev.saturating_add(1);
                            let auth_state = AuthoritativePlaybackState {
                                revision: host_rev,
                                current_track: player_snap.current_track.as_ref().map(|t| RoomTrackRef {
                                    video_id: t.id.clone(),
                                    title: t.title.clone(),
                                    artist: t.artist_display(),
                                    duration_ms: t.duration_ms,
                                    artwork_url: t.artwork_url.clone(),
                                }),
                                queue: player_snap.queue.tracks.iter().take(25).map(|t| RoomTrackRef {
                                    video_id: t.id.clone(),
                                    title: t.title.clone(),
                                    artist: t.artist_display(),
                                    duration_ms: t.duration_ms,
                                    artwork_url: t.artwork_url.clone(),
                                }).collect(),
                                is_playing: player_snap.state.is_playing(),
                                anchor_position_ms: player_snap.position_ms,
                                anchor_server_ms: unix_secs().saturating_mul(1000),
                            };
                            let _ = room.host_sync_state(auth_state).await;
                            let _ = ui_tx.send(LiveUpdate::RoomSnapshotUpdated(Some(room.snapshot())));
                        }
                    }
                    res = room.recv_next(unix_secs().saturating_mul(1000)) => {
                        match res {
                            Ok(Some(_msg)) => {
                                let snap = room.snapshot();
                                if snap.role == RoomRole::Guest {
                                    let local_snap = controller.snapshot().await;
                                    if let Some(ref rt) = snap.playback.current_track
                                        && last_guest_track_id.as_deref() != Some(rt.video_id.as_str())
                                    {
                                        last_guest_track_id = Some(rt.video_id.clone());
                                        let track = Track {
                                            id: rt.video_id.clone(),
                                            title: rt.title.clone(),
                                            artists: vec![ArtistRef {
                                                id: None,
                                                name: rt.artist.clone(),
                                            }],
                                            album: None,
                                            duration_ms: rt.duration_ms,
                                            artwork_url: rt.artwork_url.clone(),
                                            is_explicit: false,
                                            is_liked: false,
                                            is_video: false,
                                        };
                                        let _ = controller.play_track(track).await;
                                    }
                                    if snap.playback.is_playing && !local_snap.state.is_playing() {
                                        let _ = controller.resume().await;
                                    } else if !snap.playback.is_playing && local_snap.state.is_playing() {
                                        let _ = controller.pause().await;
                                    }
                                    let now_ms = unix_secs().saturating_mul(1000);
                                    if let ThreeZoneDriftAction::HardSeek(target_ms) =
                                        room.evaluate_guest_drift_three_zone(local_snap.position_ms, now_ms, false)
                                    {
                                        let _ = controller.seek_ms(target_ms).await;
                                    }
                                }
                                let _ = ui_tx.send(LiveUpdate::RoomSnapshotUpdated(Some(snap)));
                            }
                            Ok(None) => break,
                            Err(e) => {
                                let _ = ui_tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                                    "Room connection interrupted: {e}"
                                ))));
                                tokio::time::sleep(Duration::from_secs(2)).await;
                            }
                        }
                    }
                }
            }

            if let Ok(mut guard) = room_tx_slot.lock() {
                guard.take();
            }
        });
    }

    fn dispatch_cast(&self, cmd: EngineCommand) {
        let tx = self.tx.clone();
        match cmd {
            EngineCommand::StartCastDiscovery => {
                let discovery_slot = self.cast_discovery.clone();
                let _guard = self.rt.enter();
                let service = CastDiscoveryService::start();
                let mut rx = service.subscribe();
                if let Ok(mut slot) = discovery_slot.lock() {
                    *slot = Some(service);
                }
                self.rt.spawn(async move {
                    let initial = rx.borrow().clone();
                    let _ = tx.send(LiveUpdate::CastDevicesUpdated(initial));
                    while rx.changed().await.is_ok() {
                        let list = rx.borrow().clone();
                        let _ = tx.send(LiveUpdate::CastDevicesUpdated(list));
                    }
                });
            }
            EngineCommand::StopCastDiscovery => {
                if let Ok(mut slot) = self.cast_discovery.lock() {
                    if let Some(mut svc) = slot.take() {
                        svc.stop();
                    }
                }
            }
            EngineCommand::ConnectCastDevice(device) => {
                let cast = self.cast_manager.clone();
                let controller = self.controller.clone();
                self.rt.spawn(async move {
                    let snap = controller.snapshot().await;
                    let Some(track) = snap.current_track else {
                        let _ = tx.send(LiveUpdate::StatusBanner(
                            "Start playing a track before casting to a device".into(),
                        ));
                        return;
                    };
                    let meta = NowPlayingMetadata {
                        track_id: track.id.clone(),
                        title: track.title.clone(),
                        artist: track.artist_display(),
                        album: track.album.as_ref().map(|a| a.title.clone()),
                        artwork_url: track.artwork_url.clone(),
                        duration_ms: snap.duration_ms,
                    };
                    let dev_name = device.name.clone();
                    match cast
                        .connect_and_handover(
                            device,
                            &track.id,
                            meta,
                            snap.position_ms,
                            snap.duration_ms,
                        )
                        .await
                    {
                        Ok(state) => {
                            let _ = tx.send(LiveUpdate::CastSessionUpdated(state));
                            let _ = tx.send(LiveUpdate::StatusBanner(format!(
                                "Casting to {dev_name}"
                            )));
                        }
                        Err(e) => {
                            let _ = tx.send(LiveUpdate::CastSessionUpdated(
                                CastSessionState::Disconnected,
                            ));
                            let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                                "Cast connection to {dev_name} failed: {e}"
                            ))));
                        }
                    }
                });
            }
            EngineCommand::DisconnectCast => {
                let cast = self.cast_manager.clone();
                self.rt.spawn(async move {
                    match cast.disconnect().await {
                        Ok(state) => {
                            let _ = tx.send(LiveUpdate::CastSessionUpdated(state));
                            let _ = tx.send(LiveUpdate::StatusBanner(
                                "Returned audio to this device".into(),
                            ));
                        }
                        Err(e) => {
                            let _ = tx.send(LiveUpdate::CastSessionUpdated(
                                CastSessionState::Disconnected,
                            ));
                            let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                                "Cast disconnect warning: {e}"
                            ))));
                        }
                    }
                });
            }
            EngineCommand::SetCastVolume(vol) => {
                let cast = self.cast_manager.clone();
                self.rt.spawn(async move {
                    let _ = cast.set_volume(vol.clamp(0.0, 1.0)).await;
                    let _ = tx.send(LiveUpdate::CastSessionUpdated(cast.state().await));
                });
            }
            _ => unreachable!("non-cast command routed to dispatch_cast"),
        }
    }

    fn dispatch_updater(&self, cmd: EngineCommand) {
        let updater = self.updater.clone();
        let tx = self.tx.clone();
        match cmd {
            EngineCommand::CheckForUpdates {
                channel,
                skipped_version,
            } => {
                self.rt.spawn(async move {
                    let _ = tx.send(LiveUpdate::UpdaterPhaseUpdated(UpdatePhase::Checking));
                    let phase = updater
                        .check_for_updates(channel, skipped_version.as_deref())
                        .await;
                    let _ = tx.send(LiveUpdate::UpdaterPhaseUpdated(phase));
                });
            }
            EngineCommand::DownloadUpdate(info) => {
                self.rt.spawn(async move {
                    let tx_prog = tx.clone();
                    let info_for_prog = info.clone();
                    let phase = updater
                        .download_update(info, move |received_bytes, total_bytes| {
                            let _ = tx_prog.send(LiveUpdate::UpdaterPhaseUpdated(
                                UpdatePhase::Downloading {
                                    info: info_for_prog.clone(),
                                    received_bytes,
                                    total_bytes,
                                },
                            ));
                        })
                        .await;
                    let _ = tx.send(LiveUpdate::UpdaterPhaseUpdated(phase));
                });
            }
            EngineCommand::CancelUpdateDownload => {
                updater.cancel_download();
                let _ = tx.send(LiveUpdate::UpdaterPhaseUpdated(UpdatePhase::Idle));
            }
            EngineCommand::InstallUpdateOnQuit => match updater.stage_install_on_quit() {
                Ok(msg) => {
                    let _ = tx.send(LiveUpdate::StatusBanner(msg));
                }
                Err(e) => {
                    let _ = tx.send(LiveUpdate::StatusBanner(format!(
                        "Install on quit unavailable: {e}"
                    )));
                }
            },
            EngineCommand::RestartToInstallUpdate => match updater.restart_to_install() {
                Ok(msg) => {
                    let _ = tx.send(LiveUpdate::StatusBanner(msg));
                }
                Err(e) => {
                    let _ = tx.send(LiveUpdate::StatusBanner(format!(
                        "Restart install unavailable: {e}"
                    )));
                }
            },
            _ => unreachable!("non-updater command routed to dispatch_updater"),
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

enum RoomSessionInit {
    Create {
        relay_url: String,
        display_name: String,
        queue_permission: resonance_contract::QueuePermission,
    },
    Join {
        relay_url: String,
        room_code: String,
        display_name: String,
    },
}

async fn expand_autoplay_for_current(
    controller: &Arc<PlaybackController>,
    provider: &Arc<HybridMusicProvider>,
    tx: &crossbeam_channel::Sender<LiveUpdate>,
) {
    if let Some(track) = controller.snapshot().await.current_track {
        let _ = controller.expand_autoplay_queue(&track.id).await;
        fetch_and_send_related(provider, tx, track.id).await;
    }
}

async fn fetch_and_send_related(
    provider: &Arc<HybridMusicProvider>,
    tx: &crossbeam_channel::Sender<LiveUpdate>,
    video_id: String,
) {
    if let Ok(related) = provider.autoplay_next(&video_id).await {
        let _ = tx.send(LiveUpdate::RelatedTracksLoaded {
            video_id,
            tracks: related,
        });
    }
}
