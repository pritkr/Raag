//! Background engine bootstrap, service wiring, and event/telemetry pumps.

use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Sender, unbounded};
use resonance_contract::{
    AppEvent, AudioBackend, AudioTelemetry, AuthSession, CacheStore, EventBus, MediaCommand,
    SettingsV1,
};
use resonance_core::cache::{
    ArtworkCache, ChunkCache, DEFAULT_ARTWORK_DISK_CAP, DEFAULT_MAX_AUDIO_CACHE_BYTES,
    SqliteCacheStore, load_cached_library_snapshot, load_persisted_settings,
    save_persisted_settings,
};
use resonance_core::innertube::auth::redact_secrets;
use resonance_core::innertube::{InnerTubeClient, MutationWorker, RemoteConfigSyncService};
use resonance_core::local_media::LocalMediaStore;
use resonance_core::lyrics::CascadeLyricsProvider;
use resonance_core::offline::{DownloadManager, OfflineVault};
use resonance_core::playback::PlaybackController;
use resonance_core::potoken::{PoTokenPipelineConfig, build_po_token_pipeline};
use resonance_core::providers::{AtrReporter, HybridMusicProvider};
use resonance_core::proxy::LoopbackProxy;
use resonance_core::sponsorblock::HttpSponsorBlockProvider;
use resonance_core::storage::{bootstrap_st1_services, resolve_app_paths};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

#[cfg(target_os = "android")]
use super::EngineCommand;
use super::{AppEngine, LiveUpdate};
use crate::platform::{create_platform_auth_store, platform_auth_status};

impl AppEngine {
    /// Boots the production background engine on a dedicated Tokio runtime.
    pub fn boot() -> Result<Self, String> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("resonance-core")
            .build()
            .map_err(|e| format!("failed to build Tokio runtime: {e}"))?;

        let paths =
            resolve_app_paths().map_err(|e| format!("failed to resolve app paths: {e}"))?;
        let dir = paths.data_dir.clone();
        #[cfg(target_os = "android")]
        let _ = crate::platform::android::ensure_downloads_dir(&dir);
        #[cfg(not(target_os = "android"))]
        let _ = crate::platform::desktop::ensure_downloads_dir(&dir);

        let cache = Arc::new(
            SqliteCacheStore::open(&dir).map_err(|e| format!("SQLite cache open failed: {e}"))?,
        );
        let chunks = Arc::new(
            ChunkCache::new(
                &paths.cache_dir.join("chunks"),
                cache.clone(),
                DEFAULT_MAX_AUDIO_CACHE_BYTES,
            )
            .map_err(|e| format!("ChunkCache init failed: {e}"))?,
        );
        let artwork = Arc::new(
            ArtworkCache::new(&dir, 128, DEFAULT_ARTWORK_DISK_CAP)
                .map_err(|e| format!("ArtworkCache init failed: {e}"))?,
        );
        let auth = create_platform_auth_store(&dir);
        let config_base_url = env_trimmed("RESONANCE_REMOTE_CONFIG_URL");
        let st1 = bootstrap_st1_services(&paths, cache.clone(), config_base_url)
            .map_err(|e| format!("ST-1 bootstrap failed: {e}"))?;

        let inner_client = Arc::new(
            InnerTubeClient::new(cache.clone(), auth, st1.solver.clone())
                .map_err(|e| format!("InnerTubeClient init failed: {e}"))?,
        );
        if let Ok(matrix) = st1.remote_config.load_cached_matrix_sync() {
            inner_client.update_client_matrix(matrix);
        }

        // Single consolidated PO-Token acquisition pipeline.
        let po_pipeline = build_po_token_pipeline(PoTokenPipelineConfig {
            bgutil_url: env_trimmed("RESONANCE_POTOKEN_URL"),
            quickjs_script: env_trimmed("RESONANCE_POTOKEN_VM_SCRIPT")
                .and_then(|path| std::fs::read_to_string(path).ok()),
            probe_rustypipe_botguard: true,
            ..Default::default()
        });
        inner_client.set_po_token_provider(Some(po_pipeline));

        let provider = Arc::new(HybridMusicProvider::new(inner_client.clone(), cache.clone()));

        let proxy = rt.block_on(async {
            LoopbackProxy::start_with_caches(
                inner_client.clone(),
                chunks.clone(),
                Some(cache.clone()),
                Some(artwork.clone()),
            )
            .await
            .map(Arc::new)
            .map_err(|e| format!("LoopbackProxy start failed: {e}"))
        })?;
        if let Ok(local_store) = LocalMediaStore::open(&dir) {
            proxy.set_local_media_store(Arc::new(local_store));
        }

        #[cfg(target_os = "android")]
        let (audio, android_auth_rx) = {
            let bridge = Arc::new(crate::platform::android::AndroidMediaBridge::new());
            let auth_rx = bridge.take_auth_cookie_receiver();
            (bridge as Arc<dyn AudioBackend>, auth_rx)
        };
        #[cfg(not(target_os = "android"))]
        let audio: Arc<dyn AudioBackend> =
            Arc::new(crate::platform::nsh_audio::NshAudioBackend::new());

        let audio_telemetry_rx = audio.take_telemetry_receiver();
        let audio_cmd_rx = audio.take_command_receiver();

        let lyrics = Arc::new(CascadeLyricsProvider::new(
            inner_client.clone(),
            cache.clone(),
        ));
        let sponsorblock = Arc::new(HttpSponsorBlockProvider::new());
        let bus = EventBus::new(128);
        let mutations = {
            let _guard = rt.enter();
            MutationWorker::spawn(inner_client.clone(), bus.clone())
        };
        let download_manager = Arc::new(
            DownloadManager::new(
                inner_client,
                cache.clone(),
                chunks.clone(),
                artwork,
                lyrics.clone(),
                bus.clone(),
            )
            .map_err(|e| format!("DownloadManager init failed: {e}"))?,
        );
        let vault = Arc::new(OfflineVault::new(cache.clone(), download_manager));
        let _ = rt.block_on(vault.startup_recovery());

        let controller = Arc::new(
            PlaybackController::new(
                provider.clone(),
                audio,
                cache.clone(),
                proxy.clone(),
                lyrics,
                sponsorblock,
                bus.clone(),
            )
            .with_offline_store(cache.clone()),
        );

        let (tx, rx) = unbounded();

        spawn_audio_pumps(&rt, &controller, &tx, audio_telemetry_rx, audio_cmd_rx);

        #[cfg(target_os = "android")]
        if let Some(mut auth_rx) = android_auth_rx {
            rt.spawn(async move {
                while let Some(cookie_header) = auth_rx.recv().await {
                    super::dispatch(EngineCommand::SignIn(cookie_header));
                }
            });
        }

        spawn_event_bus_forwarder(&rt, &bus, &tx);

        let initial_snap = rt.block_on(controller.snapshot());
        let persisted_settings = load_persisted_settings(cache.as_ref());
        rt.block_on(controller.set_sponsorblock_enabled(persisted_settings.sponsorblock_enabled));

        let stats = cache.stats().unwrap_or_default();
        let initial_downloads = vault.list_records().unwrap_or_default();
        let current_session = provider.current_session();
        let signed_in = current_session.is_some();
        {
            let _guard = rt.enter();
            let _atr_handle = AtrReporter::spawn(bus, mutations.clone(), signed_in);
        }

        let auth_status = platform_auth_status(current_session.as_ref());
        let proxy_base = format!("http://{}", proxy.addr());
        crate::store::set_proxy_base_url(&proxy_base);
        let _ = tx.send(LiveUpdate::Ready {
            proxy_base_url: proxy_base,
            snapshot: initial_snap,
            cache_stats: stats,
            signed_in,
        });
        let _ = tx.send(LiveUpdate::SettingsLoaded(persisted_settings.clone()));
        let _ = tx.send(LiveUpdate::AuthStatus(auth_status));
        let _ = tx.send(LiveUpdate::RemoteConfigUpdated(st1.remote_config.status()));
        let _ = tx.send(LiveUpdate::PoTokenDiagnostics(
            provider.po_token_diagnostics(),
        ));
        let _ = tx.send(LiveUpdate::DownloadsLoaded(initial_downloads));
        let initial_pinned = vault.list_pinned_collections().unwrap_or_default();
        let _ = tx.send(LiveUpdate::PinnedCollectionsLoaded(initial_pinned));

        // Synchronous local library hydration (< 10ms) before touching the network.
        if let Some(cached_lib) = load_cached_library_snapshot(cache.as_ref()) {
            let _ = tx.send(LiveUpdate::LibraryLoaded(cached_lib));
        }

        if !persisted_settings.offline_mode {
            spawn_online_bootstrap(
                &rt,
                &st1.remote_config,
                &provider,
                &tx,
                current_session,
            );
        }

        let settings_tx = spawn_settings_persister(&rt, cache.clone());

        Ok(Self {
            rt,
            provider,
            _player_fetcher: st1.player_fetcher,
            remote_config: st1.remote_config,
            cache,
            chunks,
            vault,
            controller,
            mutations,
            settings_tx,
            tx,
            rx,
        })
    }
}

fn env_trimmed(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn spawn_audio_pumps(
    rt: &Runtime,
    controller: &Arc<PlaybackController>,
    tx: &Sender<LiveUpdate>,
    audio_telemetry_rx: Option<UnboundedReceiver<AudioTelemetry>>,
    audio_cmd_rx: Option<UnboundedReceiver<MediaCommand>>,
) {
    // Unified full-duplex audio telemetry pump (Desktop + Android).
    if let Some(mut tel_rx) = audio_telemetry_rx {
        let ctrl = controller.clone();
        rt.spawn(async move {
            while let Some(ev) = tel_rx.recv().await {
                let _ = ctrl.handle_audio_telemetry(ev).await;
            }
        });
    }

    // OS media key / AVRCP / MPRIS / Media3 command pump.
    if let Some(mut cmd_rx) = audio_cmd_rx {
        let ctrl = controller.clone();
        let ui_tx = tx.clone();
        rt.spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let Err(e) = ctrl.handle_media_command(cmd).await {
                    let _ = ui_tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                        "Media command failed: {e}"
                    ))));
                }
            }
        });
    }
}

fn spawn_event_bus_forwarder(rt: &Runtime, bus: &EventBus, tx: &Sender<LiveUpdate>) {
    let mut bus_rx = bus.subscribe();
    let ui_tx = tx.clone();
    rt.spawn(async move {
        while let Ok(ev) = bus_rx.recv().await {
            match ev {
                AppEvent::PlayerUpdated(snap) => {
                    let _ = ui_tx.send(LiveUpdate::PlaybackUpdated(snap));
                }
                AppEvent::LyricsLoaded { lyrics, .. } => {
                    let _ = ui_tx.send(LiveUpdate::LyricsLoaded(lyrics));
                }
                AppEvent::MutationFailed { command, reason } => {
                    let _ = ui_tx.send(LiveUpdate::MutationFailed { command, reason });
                }
                AppEvent::AuthStateChanged {
                    signed_in: false, ..
                } => {
                    let _ = ui_tx.send(LiveUpdate::AuthUpdated {
                        signed_in: false,
                        banner: "Session expired — please sign in again".into(),
                    });
                }
                AppEvent::DownloadUpdated(rec) => {
                    let _ = ui_tx.send(LiveUpdate::DownloadUpdated(rec));
                }
                AppEvent::DownloadRemoved { video_id } => {
                    let _ = ui_tx.send(LiveUpdate::DownloadRemoved(video_id));
                }
                _ => {}
            }
        }
    });
}

fn spawn_online_bootstrap(
    rt: &Runtime,
    remote_config: &Arc<RemoteConfigSyncService>,
    provider: &Arc<HybridMusicProvider>,
    tx: &Sender<LiveUpdate>,
    current_session: Option<AuthSession>,
) {
    // Background Remote Config sync.
    let rc_for_sync = remote_config.clone();
    let provider_for_rc = provider.clone();
    let tx_for_rc = tx.clone();
    rt.spawn(async move {
        if let Ok(Some(matrix)) = rc_for_sync.sync_client_matrix(false).await {
            provider_for_rc.update_client_matrix(matrix);
        }
        let _ = rc_for_sync.sync_solver_manifest(false).await;
        let _ = tx_for_rc.send(LiveUpdate::RemoteConfigUpdated(rc_for_sync.status()));
    });

    // Async network refresh + session sync when signed in.
    if let Some(session_for_sync) = current_session {
        let provider_for_lib = provider.clone();
        let tx_for_lib = tx.clone();
        rt.spawn(async move {
            let _ = provider_for_lib.sync_session(Some(&session_for_sync)).await;
            if let Ok(Some(profile)) = provider_for_lib.probe_account_profile().await {
                let _ = tx_for_lib.send(LiveUpdate::AccountProfileUpdated(Some(profile)));
                let status = platform_auth_status(provider_for_lib.current_session().as_ref());
                let _ = tx_for_lib.send(LiveUpdate::AuthStatus(status));
            }
            let snap = provider_for_lib.refresh_and_cache_library_snapshot().await;
            let _ = tx_for_lib.send(LiveUpdate::LibraryLoaded(snap));
        });
    }
}

/// Debounced background settings persistence (250ms coalesce for sliders, immediate for toggles).
fn spawn_settings_persister(
    rt: &Runtime,
    cache: Arc<SqliteCacheStore>,
) -> UnboundedSender<(SettingsV1, bool)> {
    let (settings_tx, mut settings_rx) =
        tokio::sync::mpsc::unbounded_channel::<(SettingsV1, bool)>();
    rt.spawn(async move {
        while let Some((mut latest, immediate)) = settings_rx.recv().await {
            if !immediate {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            while let Ok((newer, _)) = settings_rx.try_recv() {
                latest = newer;
            }
            save_persisted_settings(cache.as_ref(), &latest);
        }
    });
    settings_tx
}
