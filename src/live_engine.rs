//! Live Mode Background Engine Bridge (Dedicated Tokio Runtime off the NSH UI Thread).
//!
//! When Resonance runs in live mode (`--fixture` NOT enabled), this module owns the real
//! production `InnerTubeClient`, `QuickJsSolver`, `SqliteCacheStore`, `ChunkCache`,
//! `LoopbackProxy`, `CascadeLyricsProvider`, `HttpSponsorBlockProvider`, and
//! `PlaybackController` on a dedicated background Tokio runtime and streams typed updates
//! back to the NSH reactive store via a non-blocking channel.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};
use resonance_contract::{
    AccountProfile, AppEvent, AuthBackendStatus, AuthSession, CacheStats, CacheStore,
    CatalogExtProvider, DownloadRecord, ExplorePage, HomeSection, LIBRARY_SNAPSHOT_METADATA_KEY,
    LibraryCollectionRef, LibrarySnapshot, LikeAction, LyricsDocument, LyricsProvider,
    MusicProvider, MutationCommand, OfflineStore, PlayerSnapshot, Playlist, PlaylistEditOp,
    PoTokenDiagnostics, PoTokenProvider, RemoteConfigStatus, SETTINGS_METADATA_KEY,
    SETTINGS_TTL_SECS, SearchCategory, SearchResults, SettingsV1, Track, TrackId, WatchtimeReport,
};
use resonance_core::audio_dsp::DspConfig;
use resonance_core::cache::{
    ArtworkCache, ChunkCache, DEFAULT_ARTWORK_DISK_CAP, DEFAULT_MAX_AUDIO_CACHE_BYTES,
    SqliteCacheStore,
};
use resonance_core::downloads::{OfflineVault, PinnedCollection, PinnedCollectionKind};
use resonance_core::innertube::auth::redact_secrets;
use resonance_core::innertube::{
    AccountProfileProbe, InnerTubeClient, MutationWorker, RemoteConfigSyncService,
};
use resonance_core::lyrics::CascadeLyricsProvider;
use resonance_core::offline::DownloadManager;
use resonance_core::playback::PlaybackController;
use resonance_core::potoken::{
    CachingPoTokenProvider, ChainedPoTokenProvider, PlatformAttestationPoTokenProvider,
    QuickJsVmAttestationBridge, SidecarPoTokenProvider,
};
use resonance_core::providers::{AtrReporter, HybridMusicProvider};
use resonance_core::proxy::LoopbackProxy;
use resonance_core::solver::PlayerScriptFetcher;
use resonance_core::sponsorblock::HttpSponsorBlockProvider;
use resonance_core::storage::{bootstrap_st1_services, resolve_app_paths};
use tokio::runtime::Runtime;

use crate::platform::{create_platform_auth_store, platform_auth_status};

/// Typed updates pushed from the background Tokio engine to the NSH UI thread.
#[derive(Debug, Clone)]
pub enum LiveUpdate {
    Ready {
        proxy_base_url: String,
        snapshot: PlayerSnapshot,
        cache_stats: CacheStats,
        signed_in: bool,
    },
    HomeLoaded(Result<Vec<HomeSection>, String>),
    ExploreLoaded(Result<Vec<HomeSection>, String>),
    ExplorePageLoaded(Result<ExplorePage, String>),
    MoodPlaylistsLoaded {
        mood: String,
        result: Result<Vec<LibraryCollectionRef>, String>,
    },
    SearchLoaded {
        query: String,
        result: Result<SearchResults, String>,
        plays: HashMap<String, String>,
    },
    SearchSuggestionsLoaded {
        prefix: String,
        suggestions: Vec<String>,
    },
    SearchContinuationLoaded {
        query: String,
        result: Result<SearchResults, String>,
    },
    CollectionLoaded(Result<Playlist, String>),
    PlaybackUpdated(PlayerSnapshot),
    AutoplayQueueLoaded {
        seed_video_id: TrackId,
        tracks: Vec<Track>,
    },
    LyricsLoaded(Option<LyricsDocument>),
    CacheStatsUpdated(CacheStats),
    AuthUpdated {
        signed_in: bool,
        banner: String,
    },
    AuthStatus(AuthBackendStatus),
    AccountProfileUpdated(Option<AccountProfile>),
    RemoteConfigUpdated(RemoteConfigStatus),
    PoTokenDiagnostics(PoTokenDiagnostics),
    MutationFailed {
        command: MutationCommand,
        reason: String,
    },
    DownloadsLoaded(Vec<DownloadRecord>),
    DownloadUpdated(DownloadRecord),
    DownloadRemoved(TrackId),
    PinnedCollectionsLoaded(Vec<PinnedCollection>),
    LibraryLoaded(LibrarySnapshot),
    SettingsLoaded(SettingsV1),
    StatusBanner(String),
}

struct LiveRuntime {
    rt: Runtime,
    provider: Arc<InnerTubeClient>,
    hybrid: Arc<HybridMusicProvider>,
    _player_fetcher: Arc<PlayerScriptFetcher>,
    remote_config: Arc<RemoteConfigSyncService>,
    cache: Arc<SqliteCacheStore>,
    chunks: Arc<ChunkCache>,
    lyrics: Arc<CascadeLyricsProvider>,
    downloads: Arc<DownloadManager>,
    vault: Arc<OfflineVault>,
    controller: Arc<PlaybackController>,
    mutations: MutationWorker,
    settings_tx: tokio::sync::mpsc::UnboundedSender<(SettingsV1, bool)>,
    tx: Sender<LiveUpdate>,
    rx: Receiver<LiveUpdate>,
}

static LIVE_RUNTIME: OnceLock<Result<LiveRuntime, String>> = OnceLock::new();

fn init_runtime() -> Result<LiveRuntime, String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("resonance-core")
        .build()
        .map_err(|e| format!("failed to build Tokio runtime: {e}"))?;

    let paths = resolve_app_paths().map_err(|e| format!("failed to resolve app paths: {e}"))?;
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
    let config_base_url = std::env::var("RESONANCE_REMOTE_CONFIG_URL")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let st1 = bootstrap_st1_services(&paths, cache.clone(), config_base_url)
        .map_err(|e| format!("ST-1 bootstrap failed: {e}"))?;
    let solver = st1.solver.clone();
    let provider = Arc::new(
        InnerTubeClient::new(cache.clone(), auth, solver)
            .map_err(|e| format!("InnerTubeClient init failed: {e}"))?,
    );
    if let Ok(matrix) = st1.remote_config.load_cached_matrix_sync() {
        provider.update_client_matrix(matrix);
    }
    let hybrid = Arc::new(HybridMusicProvider::new(provider.clone(), cache.clone()));
    let mut po_chain: Vec<Arc<dyn PoTokenProvider>> = Vec::new();
    if let Ok(pot_url) = std::env::var("RESONANCE_POTOKEN_URL")
        && !pot_url.trim().is_empty()
    {
        match SidecarPoTokenProvider::new(pot_url.trim()) {
            Ok(sidecar) => {
                po_chain.push(Arc::new(sidecar));
            }
            Err(e) => {
                log::warn!(
                    "ignoring invalid RESONANCE_POTOKEN_URL: {}",
                    redact_secrets(&e.to_string())
                );
            }
        }
    }
    if let Ok(vm_script_path) = std::env::var("RESONANCE_POTOKEN_VM_SCRIPT")
        && !vm_script_path.trim().is_empty()
        && let Ok(script_source) = std::fs::read_to_string(vm_script_path.trim())
    {
        po_chain.push(Arc::new(PlatformAttestationPoTokenProvider::new(Arc::new(
            QuickJsVmAttestationBridge::new(script_source),
        ))));
    }
    let chained_po = ChainedPoTokenProvider::new(po_chain);
    let cached_po = CachingPoTokenProvider::new(chained_po).into_arc();
    provider.set_po_token_provider(Some(cached_po));
    let artwork_for_proxy = artwork.clone();
    let proxy = rt.block_on(async {
        LoopbackProxy::start_with_caches(
            provider.clone(),
            chunks.clone(),
            Some(cache.clone()),
            Some(artwork_for_proxy),
        )
        .await
        .map(Arc::new)
        .map_err(|e| format!("LoopbackProxy start failed: {e}"))
    })?;

    #[cfg(target_os = "android")]
    let (audio, android_telemetry_rx) = {
        let bridge = Arc::new(crate::platform::android::AndroidMediaBridge::new());
        let rx = bridge.take_telemetry_receiver();
        (bridge as Arc<dyn resonance_contract::AudioBackend>, rx)
    };
    #[cfg(not(target_os = "android"))]
    let audio: Arc<dyn resonance_contract::AudioBackend> =
        Arc::new(crate::platform::nsh_audio::NshAudioBackend::new());
    let audio_cmd_rx = audio.take_command_receiver();
    let lyrics = Arc::new(CascadeLyricsProvider::new(provider.clone(), cache.clone()));
    let sponsorblock = Arc::new(HttpSponsorBlockProvider::new());
    let bus = resonance_contract::EventBus::new(128);
    let mutations = {
        let _guard = rt.enter();
        MutationWorker::spawn(provider.clone(), bus.clone())
    };
    let downloads = Arc::new(
        DownloadManager::new(
            provider.clone(),
            cache.clone(),
            chunks.clone(),
            artwork,
            lyrics.clone(),
            bus.clone(),
        )
        .map_err(|e| format!("DownloadManager init failed: {e}"))?,
    );
    let _ = rt.block_on(downloads.startup_recovery());
    let vault = Arc::new(OfflineVault::new(cache.clone(), downloads.clone()));

    let controller = Arc::new(
        PlaybackController::new(
            hybrid.clone(),
            audio,
            cache.clone(),
            proxy.clone(),
            lyrics.clone(),
            sponsorblock,
            bus.clone(),
        )
        .with_offline_store(cache.clone()),
    );

    let (tx, rx) = unbounded();
    #[cfg(not(target_os = "android"))]
    {
        let ctrl_for_audio = controller.clone();
        let tx_for_audio = tx.clone();
        let rt_handle = rt.handle().clone();
        let _ = std::thread::Builder::new()
            .name("resonance-audio-telemetry".to_owned())
            .spawn(move || {
                let bus = crate::platform::nsh_audio::nsh_audio_bus();
                while let Ok(ev) = bus.telemetry_rx.recv() {
                    if rt_handle
                        .block_on(ctrl_for_audio.handle_audio_telemetry(ev))
                        .is_ok()
                    {
                        let snap = rt_handle.block_on(ctrl_for_audio.snapshot());
                        let _ = tx_for_audio.send(LiveUpdate::PlaybackUpdated(snap));
                    }
                }
            });
    }
    #[cfg(target_os = "android")]
    if let Some(mut android_rx) = android_telemetry_rx {
        let ctrl_for_android = controller.clone();
        let tx_for_android = tx.clone();
        rt.spawn(async move {
            use crate::platform::android::AndroidTelemetryEvent;
            use resonance_core::playback::AudioTelemetry;
            while let Some(ev) = android_rx.recv().await {
                match ev {
                    AndroidTelemetryEvent::Progress {
                        generation,
                        position_ms,
                        ..
                    } => {
                        if ctrl_for_android
                            .handle_audio_telemetry(AudioTelemetry::Progress {
                                generation,
                                position_ms,
                            })
                            .await
                            .is_ok()
                        {
                            let snap = ctrl_for_android.snapshot().await;
                            let _ = tx_for_android.send(LiveUpdate::PlaybackUpdated(snap));
                        }
                    }
                    AndroidTelemetryEvent::PlaybackState {
                        generation,
                        state_code,
                        error_msg,
                    } => {
                        let state = match state_code {
                            1 => resonance_contract::PlayerState::Buffering,
                            2 => resonance_contract::PlayerState::Playing,
                            3 => resonance_contract::PlayerState::Paused,
                            _ => resonance_contract::PlayerState::Idle,
                        };
                        if let Some(msg) = error_msg {
                            let _ = ctrl_for_android
                                .handle_audio_telemetry(AudioTelemetry::Error {
                                    generation,
                                    message: msg,
                                })
                                .await;
                        } else {
                            let _ = ctrl_for_android
                                .handle_audio_telemetry(AudioTelemetry::StateChanged {
                                    generation,
                                    state,
                                })
                                .await;
                        }
                        let snap = ctrl_for_android.snapshot().await;
                        let _ = tx_for_android.send(LiveUpdate::PlaybackUpdated(snap));
                    }
                    AndroidTelemetryEvent::AuthCookiesCaptured { cookie_header } => {
                        dispatch_sign_in(cookie_header);
                    }
                }
            }
        });
    }
    if let Some(mut cmd_rx) = audio_cmd_rx {
        let ctrl_for_media = controller.clone();
        let tx_for_media = tx.clone();
        rt.spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match ctrl_for_media.handle_media_command(cmd).await {
                    Ok(()) => {
                        let snap = ctrl_for_media.snapshot().await;
                        let _ = tx_for_media.send(LiveUpdate::PlaybackUpdated(snap));
                    }
                    Err(e) => {
                        let _ = tx_for_media.send(LiveUpdate::StatusBanner(redact_secrets(
                            &format!("Media command failed: {e}"),
                        )));
                    }
                }
            }
        });
    }
    {
        let mut bus_rx = bus.subscribe();
        let ui_tx = tx.clone();
        rt.spawn(async move {
            while let Ok(ev) = bus_rx.recv().await {
                match ev {
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
    let initial_snap = rt.block_on(controller.snapshot());
    let persisted_settings = load_persisted_settings(cache.as_ref());
    rt.block_on(controller.set_sponsorblock_enabled(persisted_settings.sponsorblock_enabled));

    let stats = cache.stats().unwrap_or_default();
    let initial_downloads = cache.list_downloads().unwrap_or_default();
    let current_session = provider.current_session();
    let signed_in = current_session.is_some();
    {
        let _guard = rt.enter();
        let _atr_handle = AtrReporter::spawn(bus.clone(), mutations.clone(), signed_in);
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

    // H0.1 Step 1: Synchronous local library hydration (< 10ms) before touching the network.
    if let Some(cached_lib) = load_cached_library_snapshot(cache.as_ref()) {
        let _ = tx.send(LiveUpdate::LibraryLoaded(cached_lib));
    }
    // ST-1: Background Remote Config sync + AccountProfileProbe when online.
    if !persisted_settings.offline_mode {
        let rc_for_sync = st1.remote_config.clone();
        let provider_for_rc = provider.clone();
        let tx_for_rc = tx.clone();
        rt.spawn(async move {
            if let Ok(Some(matrix)) = rc_for_sync.sync_client_matrix(false).await {
                provider_for_rc.update_client_matrix(matrix);
            }
            let _ = rc_for_sync.sync_solver_manifest(false).await;
            let _ = tx_for_rc.send(LiveUpdate::RemoteConfigUpdated(rc_for_sync.status()));
        });
    }
    // H0.1 Step 2: Async network refresh + session sync when signed in and not in offline mode.
    if signed_in && !persisted_settings.offline_mode {
        let provider_for_lib = provider.clone();
        let hybrid_for_lib = hybrid.clone();
        let cache_for_lib = cache.clone();
        let tx_for_lib = tx.clone();
        let session_for_sync = current_session.clone();
        rt.spawn(async move {
            let _ = hybrid_for_lib.sync_session(session_for_sync.as_ref()).await;
            if let Ok(profile) =
                AccountProfileProbe::probe_and_update(provider_for_lib.as_ref()).await
            {
                let _ = tx_for_lib.send(LiveUpdate::AccountProfileUpdated(Some(profile)));
                let status = platform_auth_status(provider_for_lib.current_session().as_ref());
                let _ = tx_for_lib.send(LiveUpdate::AuthStatus(status));
            }
            let snap =
                refresh_library_snapshot(hybrid_for_lib.as_ref(), cache_for_lib.as_ref()).await;
            let _ = tx_for_lib.send(LiveUpdate::LibraryLoaded(snap));
        });
    }

    // H0.2: Debounced background settings persistence (250ms coalesce for sliders, immediate for toggles).
    let (settings_tx, mut settings_rx) =
        tokio::sync::mpsc::unbounded_channel::<(SettingsV1, bool)>();
    {
        let cache_for_settings = cache.clone();
        rt.spawn(async move {
            while let Some((mut latest, immediate)) = settings_rx.recv().await {
                if !immediate {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    while let Ok((newer, _)) = settings_rx.try_recv() {
                        latest = newer;
                    }
                } else {
                    while let Ok((newer, _)) = settings_rx.try_recv() {
                        latest = newer;
                    }
                }
                save_persisted_settings(cache_for_settings.as_ref(), &latest);
            }
        });
    }

    Ok(LiveRuntime {
        rt,
        provider,
        hybrid,
        _player_fetcher: st1.player_fetcher,
        remote_config: st1.remote_config,
        cache,
        chunks,
        lyrics,
        downloads,
        vault,
        controller,
        mutations,
        settings_tx,
        tx,
        rx,
    })
}

fn runtime() -> Result<&'static LiveRuntime, &'static str> {
    match LIVE_RUNTIME.get_or_init(init_runtime) {
        Ok(rt) => Ok(rt),
        Err(err) => Err(err.as_str()),
    }
}

pub fn try_drain_updates() -> Vec<LiveUpdate> {
    let Ok(rt) = runtime() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    while let Ok(ev) = rt.rx.try_recv() {
        out.push(ev);
    }
    out
}

pub fn dispatch_load_home() {
    let rt = match runtime() {
        Ok(rt) => rt,
        Err(err) => {
            crate::store::set_home_init_error(err.to_owned());
            return;
        }
    };
    let hybrid = rt.hybrid.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let res = hybrid.home().await.map_err(|e| e.to_string());
        let _ = tx.send(LiveUpdate::HomeLoaded(res));
    });
}

pub fn dispatch_load_explore() {
    let Ok(rt) = runtime() else { return };
    let hybrid = rt.hybrid.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let res = CatalogExtProvider::explore(hybrid.as_ref())
            .await
            .map_err(|e| e.to_string());
        let _ = tx.send(LiveUpdate::ExplorePageLoaded(res));
    });
}

pub fn dispatch_load_mood_playlists(mood: String, params: String) {
    let Ok(rt) = runtime() else { return };
    let hybrid = rt.hybrid.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let res = hybrid
            .mood_playlists(&params)
            .await
            .map_err(|e| e.to_string());
        let _ = tx.send(LiveUpdate::MoodPlaylistsLoaded { mood, result: res });
    });
}

pub fn dispatch_search_suggestions(prefix: String) {
    let Ok(rt) = runtime() else { return };
    if prefix.trim().is_empty() {
        return;
    }
    let hybrid = rt.hybrid.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        if let Ok(suggestions) = hybrid.search_suggestions(&prefix).await {
            let _ = tx.send(LiveUpdate::SearchSuggestionsLoaded {
                prefix,
                suggestions,
            });
        }
    });
}

pub fn dispatch_search_continuation(query: String, category: SearchCategory, continuation: String) {
    let Ok(rt) = runtime() else { return };
    let hybrid = rt.hybrid.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let res = hybrid
            .search_categorized(&query, category, Some(&continuation))
            .await
            .map_err(|e| e.to_string());
        let _ = tx.send(LiveUpdate::SearchContinuationLoaded { query, result: res });
    });
}

pub fn dispatch_update_dsp_config(config: DspConfig) {
    #[cfg(not(target_os = "android"))]
    crate::platform::nsh_audio::nsh_audio_bus().send_control(
        crate::platform::nsh_audio::NshAudioControl::UpdateDsp(config),
    );
    #[cfg(target_os = "android")]
    let _ = config;
}

pub fn dispatch_refresh_remote_config() {
    let Ok(rt) = runtime() else { return };
    let rc = rt.remote_config.clone();
    let provider = rt.provider.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        if let Ok(Some(matrix)) = rc.sync_client_matrix(true).await {
            provider.update_client_matrix(matrix);
        }
        let _ = rc.sync_solver_manifest(true).await;
        let _ = tx.send(LiveUpdate::RemoteConfigUpdated(rc.status()));
    });
}

pub fn dispatch_search(query: String, params: Option<&'static str>) {
    let rt = match runtime() {
        Ok(rt) => rt,
        Err(err) => {
            crate::store::set_search_init_error(query, err.to_owned());
            return;
        }
    };
    let provider = rt.provider.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        match provider.search_with_params_and_plays(&query, params).await {
            Ok((res, plays)) => {
                let _ = tx.send(LiveUpdate::SearchLoaded {
                    query,
                    result: Ok(res),
                    plays,
                });
            }
            Err(e) => {
                let _ = tx.send(LiveUpdate::SearchLoaded {
                    query,
                    result: Err(e.to_string()),
                    plays: HashMap::new(),
                });
            }
        }
    });
}

pub fn dispatch_browse_collection(browse_id: String) {
    let Ok(rt) = runtime() else { return };
    let provider = rt.provider.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let res = provider
            .browse_collection(&browse_id)
            .await
            .map_err(|e| e.to_string());
        let _ = tx.send(LiveUpdate::CollectionLoaded(res));
    });
}

pub fn dispatch_play_track(track: Track) {
    let Ok(rt) = runtime() else { return };
    let controller = rt.controller.clone();
    let provider = rt.provider.clone();
    let lyrics = rt.lyrics.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let track_for_lyrics = track.clone();
        let seed_id = track.id.clone();
        match controller.play_track(track).await {
            Ok(()) => {
                let snap = controller.snapshot().await;
                let _ = tx.send(LiveUpdate::PlaybackUpdated(snap));
            }
            Err(e) => {
                let _ = tx.send(LiveUpdate::StatusBanner(format!("Playback failed: {e}")));
            }
        }
        let _ = tx.send(LiveUpdate::PoTokenDiagnostics(
            provider.po_token_diagnostics(),
        ));
        let (lyr_res, next_res) = tokio::join!(
            lyrics.fetch_lyrics(&track_for_lyrics),
            provider.autoplay_next(&seed_id)
        );
        let _ = tx.send(LiveUpdate::LyricsLoaded(lyr_res.ok().flatten()));
        if let Ok(next_tracks) = next_res
            && !next_tracks.is_empty()
        {
            let _ = tx.send(LiveUpdate::AutoplayQueueLoaded {
                seed_video_id: seed_id,
                tracks: next_tracks,
            });
        }
    });
}

pub fn load_persisted_settings(cache: &dyn CacheStore) -> SettingsV1 {
    let now = resonance_core::unix_secs();
    match cache.get_metadata(SETTINGS_METADATA_KEY, now) {
        Ok(Some((json, _))) => match serde_json::from_str::<SettingsV1>(&json) {
            Ok(mut s) => {
                s.volume = s.volume.clamp(0.0, 1.0);
                s
            }
            Err(e) => {
                log::warn!("corrupt persisted settings: {e}; falling back to defaults");
                SettingsV1::default()
            }
        },
        _ => SettingsV1::default(),
    }
}

pub fn save_persisted_settings(cache: &dyn CacheStore, settings: &SettingsV1) {
    if let Ok(json) = serde_json::to_string(settings) {
        let now = resonance_core::unix_secs();
        if let Err(e) = cache.put_metadata(SETTINGS_METADATA_KEY, &json, SETTINGS_TTL_SECS, now) {
            log::warn!("failed to persist settings: {e}");
        }
    }
}

pub fn dispatch_persist_settings(settings: SettingsV1, immediate: bool) {
    let Ok(rt) = runtime() else { return };
    let _ = rt.settings_tx.send((settings, immediate));
}

pub fn load_cached_library_snapshot(cache: &dyn CacheStore) -> Option<LibrarySnapshot> {
    let now = resonance_core::unix_secs();
    let mut snap = cache
        .get_metadata(LIBRARY_SNAPSHOT_METADATA_KEY, now)
        .ok()
        .flatten()
        .and_then(|(json, _)| serde_json::from_str::<LibrarySnapshot>(&json).ok())?;

    if let Ok(local_history) = cache.load_history(200) {
        let mut seen: HashSet<(String, u64)> = snap
            .history
            .iter()
            .map(|t| (t.id.clone(), snap.fetched_at_unix_secs / 60))
            .collect();
        for entry in local_history {
            let bucket = entry.played_at_unix_secs / 60;
            if seen.insert((entry.track.id.clone(), bucket)) {
                snap.history.push(entry.track);
            }
        }
    }
    Some(snap)
}

pub async fn refresh_library_snapshot(
    provider: &dyn MusicProvider,
    cache: &dyn CacheStore,
) -> LibrarySnapshot {
    let (liked_res, playlists_res) = tokio::join!(
        provider.liked_songs(),
        provider.browse_collection("FEmusic_liked_playlists"),
    );

    let liked_songs = liked_res.unwrap_or_default();
    let playlists = match playlists_res {
        Ok(pl) => pl
            .tracks
            .into_iter()
            .map(|t| {
                let subtitle = t.artist_display();
                LibraryCollectionRef {
                    browse_id: t.id.clone(),
                    playlist_id: Some(t.id),
                    title: t.title,
                    subtitle,
                    artwork_url: t.artwork_url,
                    track_count: None,
                }
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    let now = resonance_core::unix_secs();
    let mut history = Vec::new();
    let mut seen = HashSet::new();
    if let Ok(local_history) = cache.load_history(200) {
        for entry in local_history {
            let bucket = entry.played_at_unix_secs / 60;
            if seen.insert((entry.track.id.clone(), bucket)) {
                history.push(entry.track);
            }
        }
    }

    let snap = LibrarySnapshot {
        liked_songs,
        playlists,
        albums: Vec::new(),
        artists: Vec::new(),
        history,
        podcasts: Vec::new(),
        uploads: Vec::new(),
        fetched_at_unix_secs: now,
    };

    if let Ok(json) = serde_json::to_string(&snap) {
        let _ = cache.put_metadata(LIBRARY_SNAPSHOT_METADATA_KEY, &json, 300, now);
    }
    snap
}

pub fn dispatch_sign_in(cookie_header: String) {
    let Ok(rt) = runtime() else { return };
    let provider = rt.provider.clone();
    let hybrid = rt.hybrid.clone();
    let cache = rt.cache.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let session = AuthSession {
            cookie_header,
            auth_user: 0,
            page_id: None,
            visitor_data: provider.current_visitor_data(),
            account_name: None,
            is_premium: false,
        };
        match provider.sign_in(session) {
            Ok(()) => {
                let _ = hybrid
                    .sync_session(provider.current_session().as_ref())
                    .await;
                if let Ok(profile) = AccountProfileProbe::probe_and_update(provider.as_ref()).await
                {
                    let _ = tx.send(LiveUpdate::AccountProfileUpdated(Some(profile)));
                }
                let status = platform_auth_status(provider.current_session().as_ref());
                let _ = tx.send(LiveUpdate::AuthStatus(status));
                let _ = tx.send(LiveUpdate::AuthUpdated {
                    signed_in: true,
                    banner: "Signed in securely (stored in OS Keystore)".into(),
                });
                let snap = refresh_library_snapshot(hybrid.as_ref(), cache.as_ref()).await;
                let _ = tx.send(LiveUpdate::LibraryLoaded(snap));
            }
            Err(e) => {
                let _ = tx.send(LiveUpdate::StatusBanner(redact_secrets(&format!(
                    "Sign-in failed: {e}"
                ))));
            }
        }
    });
}

pub fn dispatch_sign_out() {
    let Ok(rt) = runtime() else { return };
    let provider = rt.provider.clone();
    let hybrid = rt.hybrid.clone();
    let cache = rt.cache.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let _ = provider.sign_out();
        let _ = hybrid.sync_session(None).await;
        let _ = cache.evict_metadata(LIBRARY_SNAPSHOT_METADATA_KEY);
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

pub fn dispatch_clear_caches() {
    let Ok(rt) = runtime() else { return };
    let cache = rt.cache.clone();
    let chunks = rt.chunks.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let _ = chunks.clear_all();
        let _ = cache.clear_all_caches();
        let stats = cache.stats().unwrap_or_default();
        let _ = tx.send(LiveUpdate::CacheStatsUpdated(stats));
    });
}

pub fn dispatch_set_like(video_id: TrackId, action: LikeAction) {
    let Ok(rt) = runtime() else { return };
    let cmd = MutationCommand::SetLike { video_id, action };
    if let Err(e) = rt.mutations.enqueue(cmd.clone()) {
        let _ = rt.tx.send(LiveUpdate::MutationFailed {
            command: cmd,
            reason: e.to_string(),
        });
    }
}

pub fn dispatch_edit_playlist(playlist_id: String, op: PlaylistEditOp) {
    let Ok(rt) = runtime() else { return };
    let cmd = MutationCommand::EditPlaylist { playlist_id, op };
    if let Err(e) = rt.mutations.enqueue(cmd.clone()) {
        let _ = rt.tx.send(LiveUpdate::MutationFailed {
            command: cmd,
            reason: e.to_string(),
        });
    }
}

pub fn dispatch_report_atr(report: WatchtimeReport) {
    let Ok(rt) = runtime() else { return };
    let cmd = MutationCommand::ReportAtr(report);
    if let Err(e) = rt.mutations.enqueue(cmd.clone()) {
        let _ = rt.tx.send(LiveUpdate::MutationFailed {
            command: cmd,
            reason: e.to_string(),
        });
    }
}

pub fn dispatch_enqueue_download(track: Track) {
    let Ok(rt) = runtime() else { return };
    let downloads = rt.downloads.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        if let Err(e) = downloads.enqueue(track).await {
            let _ = tx.send(LiveUpdate::StatusBanner(format!("Download failed: {e}")));
        }
    });
}

pub fn dispatch_pause_download(video_id: TrackId) {
    let Ok(rt) = runtime() else { return };
    let downloads = rt.downloads.clone();
    rt.rt.spawn(async move {
        let _ = downloads.pause(&video_id).await;
    });
}

pub fn dispatch_resume_download(video_id: TrackId) {
    let Ok(rt) = runtime() else { return };
    let downloads = rt.downloads.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        if let Err(e) = downloads.resume(&video_id).await {
            let _ = tx.send(LiveUpdate::StatusBanner(format!("Resume failed: {e}")));
        }
    });
}

pub fn dispatch_cancel_download(video_id: TrackId) {
    let Ok(rt) = runtime() else { return };
    let downloads = rt.downloads.clone();
    rt.rt.spawn(async move {
        let _ = downloads.cancel(&video_id).await;
    });
}

pub fn dispatch_delete_download(video_id: TrackId) {
    let Ok(rt) = runtime() else { return };
    let downloads = rt.downloads.clone();
    rt.rt.spawn(async move {
        let _ = downloads.delete(&video_id).await;
    });
}

pub fn dispatch_pin_collection(
    id: String,
    kind: PinnedCollectionKind,
    title: String,
    subtitle: Option<String>,
    artwork_url: Option<String>,
    tracks: Vec<Track>,
) {
    let Ok(rt) = runtime() else { return };
    let vault = rt.vault.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
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
            let _ = tx.send(LiveUpdate::StatusBanner(format!("Pin failed: {e}")));
        } else if let Ok(list) = vault.list_pinned_collections() {
            let _ = tx.send(LiveUpdate::PinnedCollectionsLoaded(list));
        }
    });
}

pub fn dispatch_unpin_collection(collection_id: String) {
    let Ok(rt) = runtime() else { return };
    let vault = rt.vault.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let _ = vault.unpin_collection(&collection_id);
        if let Ok(list) = vault.list_pinned_collections() {
            let _ = tx.send(LiveUpdate::PinnedCollectionsLoaded(list));
        }
    });
}
