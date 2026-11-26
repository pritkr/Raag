//! Live Mode Background Engine (`AppEngine`) running on a dedicated Tokio runtime
//! off the NSH UI thread.
//!
//! Deep facade architecture:
//! - Holds unified facades only: [`HybridMusicProvider`], [`OfflineVault`],
//!   [`SqliteCacheStore`], [`PlaybackController`], and [`MutationWorker`].
//! - Consumes full-duplex [`AudioBackend::take_telemetry_receiver`] and
//!   [`AudioBackend::take_command_receiver`] identically on Desktop and Android
//!   (no blocking OS telemetry threads or platform opcode switches).
//! - Forwards [`AppEvent`]s from `EventBus` (`PlayerUpdated`, `LyricsLoaded`,
//!   `DownloadUpdated`, `DownloadRemoved`, `MutationFailed`, `AuthStateChanged`)
//!   directly into [`LiveUpdate`] without duplicating lyrics fetches or snapshot polling.
//! - Exposes a single typed [`EngineCommand`] dispatch seam.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};
use resonance_contract::{
    AccountProfile, AppEvent, AudioBackend, AuthBackendStatus, AuthSession, CacheStats, CacheStore,
    CatalogExtProvider, DownloadRecord, ExplorePage, HomeSection, LIBRARY_SNAPSHOT_METADATA_KEY,
    LibraryCollectionRef, LibrarySnapshot, LikeAction, LyricsDocument, MusicProvider,
    MutationCommand, PlayerSnapshot, Playlist, PlaylistEditOp, PoTokenDiagnostics,
    RemoteConfigStatus, RepeatMode, SETTINGS_METADATA_KEY, SETTINGS_TTL_SECS, SearchCategory,
    SearchResults, SettingsV1, Track, TrackId, WatchtimeReport,
};
use resonance_core::cache::{
    ArtworkCache, ChunkCache, DEFAULT_ARTWORK_DISK_CAP, DEFAULT_MAX_AUDIO_CACHE_BYTES,
    SqliteCacheStore,
};
use resonance_core::downloads::{OfflineVault, PinnedCollection, PinnedCollectionKind};
use resonance_core::innertube::auth::redact_secrets;
use resonance_core::innertube::{
    AccountProfileProbe, InnerTubeClient, MutationWorker, RemoteConfigSyncService,
};
use resonance_core::local_media::LocalMediaStore;
use resonance_core::lyrics::CascadeLyricsProvider;
use resonance_core::offline::DownloadManager;
use resonance_core::playback::PlaybackController;
use resonance_core::potoken::{PoTokenPipelineConfig, build_po_token_pipeline};
use resonance_core::providers::{AtrReporter, HybridMusicProvider};
use resonance_core::proxy::LoopbackProxy;
use resonance_core::solver::PlayerScriptFetcher;
use resonance_core::sponsorblock::HttpSponsorBlockProvider;
use resonance_core::storage::{bootstrap_st1_services, resolve_app_paths};
use tokio::runtime::Runtime;

use crate::platform::{create_platform_auth_store, platform_auth_status};

/// Typed commands dispatched from the NSH UI store to [`AppEngine`].
#[derive(Debug, Clone)]
pub enum EngineCommand {
    // Catalog & Search
    LoadHome,
    LoadExplore,
    LoadMoodPlaylists {
        mood: String,
        params: String,
    },
    Search {
        query: String,
        params: Option<&'static str>,
    },
    SearchSuggestions {
        prefix: String,
    },
    SearchContinuation {
        query: String,
        category: SearchCategory,
        continuation: String,
    },
    BrowseCollection {
        browse_id: String,
    },

    // Playback & Queue
    PlayTrack(Track),
    PlayQueue {
        tracks: Vec<Track>,
        start_index: usize,
        shuffle: bool,
    },
    PlayNext(Track),
    PlayLast(Track),
    NextTrack,
    PreviousTrack,
    JumpQueue(usize),
    MoveQueueItem {
        from: usize,
        to: usize,
    },
    RemoveQueueItem(usize),
    ClearQueue,
    TogglePlayPause,
    SeekMs(u64),
    SetVolume(f32),
    SetShuffle(bool),
    SetRepeatMode(RepeatMode),
    SetAutoplayRadio(bool),

    // Auth, Config, Cache & Settings
    SignIn(String),
    SignOut,
    RefreshRemoteConfig,
    ClearCaches,
    PersistSettings {
        settings: SettingsV1,
        immediate: bool,
    },

    // Optimistic Mutations
    SetLike {
        video_id: TrackId,
        action: LikeAction,
    },
    EditPlaylist {
        playlist_id: String,
        op: PlaylistEditOp,
    },
    ReportAtr(WatchtimeReport),

    // Offline Vault
    EnqueueDownload(Track),
    PauseDownload(TrackId),
    ResumeDownload(TrackId),
    CancelDownload(TrackId),
    DeleteDownload(TrackId),
    PinCollection {
        id: String,
        kind: PinnedCollectionKind,
        title: String,
        subtitle: Option<String>,
        artwork_url: Option<String>,
        tracks: Vec<Track>,
    },
    UnpinCollection(String),
}

/// Typed updates pushed from [`AppEngine`] to the NSH UI thread.
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

/// Instantiable background engine coordinating network, storage, proxy, playback, and downloads.
pub struct AppEngine {
    rt: Runtime,
    provider: Arc<HybridMusicProvider>,
    _player_fetcher: Arc<PlayerScriptFetcher>,
    remote_config: Arc<RemoteConfigSyncService>,
    cache: Arc<SqliteCacheStore>,
    chunks: Arc<ChunkCache>,
    vault: Arc<OfflineVault>,
    controller: Arc<PlaybackController>,
    mutations: MutationWorker,
    settings_tx: tokio::sync::mpsc::UnboundedSender<(SettingsV1, bool)>,
    tx: Sender<LiveUpdate>,
    rx: Receiver<LiveUpdate>,
}

static APP_ENGINE: OnceLock<Result<AppEngine, String>> = OnceLock::new();

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
        let config_base_url = std::env::var("RESONANCE_REMOTE_CONFIG_URL")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
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
            bgutil_url: std::env::var("RESONANCE_POTOKEN_URL")
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty()),
            quickjs_script: std::env::var("RESONANCE_POTOKEN_VM_SCRIPT")
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
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
        let bus = resonance_contract::EventBus::new(128);
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

        // Unified full-duplex audio telemetry pump (Desktop + Android).
        if let Some(mut tel_rx) = audio_telemetry_rx {
            let ctrl = controller.clone();
            rt.spawn(async move {
                while let Some(ev) = tel_rx.recv().await {
                    let _ = ctrl.handle_audio_telemetry(ev).await;
                }
            });
        }

        // Android WebView auth cookies capture channel.
        #[cfg(target_os = "android")]
        if let Some(mut auth_rx) = android_auth_rx {
            rt.spawn(async move {
                while let Some(cookie_header) = auth_rx.recv().await {
                    dispatch(EngineCommand::SignIn(cookie_header));
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

        // Single authoritative EventBus -> LiveUpdate forwarder.
        {
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

        // Background Remote Config sync when online.
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

        // Async network refresh + session sync when signed in and online.
        if signed_in && !persisted_settings.offline_mode {
            let provider_for_lib = provider.clone();
            let cache_for_lib = cache.clone();
            let tx_for_lib = tx.clone();
            let session_for_sync = current_session.clone();
            rt.spawn(async move {
                let _ = provider_for_lib.sync_session(session_for_sync.as_ref()).await;
                if let Some(inner) = provider_for_lib.inner_client()
                    && let Ok(profile) = AccountProfileProbe::probe_and_update(inner.as_ref()).await
                {
                    let _ = tx_for_lib.send(LiveUpdate::AccountProfileUpdated(Some(profile)));
                    let status = platform_auth_status(provider_for_lib.current_session().as_ref());
                    let _ = tx_for_lib.send(LiveUpdate::AuthStatus(status));
                }
                let snap =
                    refresh_library_snapshot(provider_for_lib.as_ref(), cache_for_lib.as_ref())
                        .await;
                let _ = tx_for_lib.send(LiveUpdate::LibraryLoaded(snap));
            });
        }

        // Debounced background settings persistence (250ms coalesce for sliders, immediate for toggles).
        let (settings_tx, mut settings_rx) =
            tokio::sync::mpsc::unbounded_channel::<(SettingsV1, bool)>();
        {
            let cache_for_settings = cache.clone();
            rt.spawn(async move {
                while let Some((mut latest, immediate)) = settings_rx.recv().await {
                    if !immediate {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                    while let Ok((newer, _)) = settings_rx.try_recv() {
                        latest = newer;
                    }
                    save_persisted_settings(cache_for_settings.as_ref(), &latest);
                }
            });
        }

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

    /// Drains all pending [`LiveUpdate`]s without blocking.
    pub fn try_drain_updates(&self) -> Vec<LiveUpdate> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            out.push(ev);
        }
        out
    }

    /// Dispatches a typed [`EngineCommand`] onto the background Tokio runtime.
    pub fn dispatch(&self, cmd: EngineCommand) {
        match cmd {
            EngineCommand::LoadHome => {
                let provider = self.provider.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let res = provider.home().await.map_err(|e| e.to_string());
                    let _ = tx.send(LiveUpdate::HomeLoaded(res));
                });
            }
            EngineCommand::LoadExplore => {
                let provider = self.provider.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let res = CatalogExtProvider::explore(provider.as_ref())
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(LiveUpdate::ExplorePageLoaded(res));
                });
            }
            EngineCommand::LoadMoodPlaylists { mood, params } => {
                let provider = self.provider.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let res = provider
                        .mood_playlists(&params)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(LiveUpdate::MoodPlaylistsLoaded { mood, result: res });
                });
            }
            EngineCommand::Search { query, params } => {
                let provider = self.provider.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
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
            EngineCommand::SearchSuggestions { prefix } => {
                if prefix.trim().is_empty() {
                    return;
                }
                let provider = self.provider.clone();
                let tx = self.tx.clone();
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
                query,
                category,
                continuation,
            } => {
                let provider = self.provider.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let res = provider
                        .search_categorized(&query, category, Some(&continuation))
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(LiveUpdate::SearchContinuationLoaded { query, result: res });
                });
            }
            EngineCommand::BrowseCollection { browse_id } => {
                let provider = self.provider.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let res = provider
                        .browse_collection(&browse_id)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(LiveUpdate::CollectionLoaded(res));
                });
            }

            // Playback & Queue (`PlaybackController` publishes `PlayerUpdated` & `LyricsLoaded` on EventBus)
            EngineCommand::PlayTrack(track) => {
                let controller = self.controller.clone();
                let provider = self.provider.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let seed_id = track.id.clone();
                    if let Err(e) = controller.play_track(track).await {
                        let _ = tx.send(LiveUpdate::StatusBanner(format!("Playback failed: {e}")));
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
                let controller = self.controller.clone();
                let provider = self.provider.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    if let Err(e) = controller.play_tracks(tracks, start_index).await {
                        let _ = tx.send(LiveUpdate::StatusBanner(format!("Playback failed: {e}")));
                        return;
                    }
                    if shuffle {
                        controller.set_shuffle(true).await;
                    }
                    let _ = tx.send(LiveUpdate::PoTokenDiagnostics(
                        provider.po_token_diagnostics(),
                    ));
                    let snap = controller.snapshot().await;
                    if let Some(t) = snap.current_track {
                        let _ = controller.expand_autoplay_queue(&t.id).await;
                    }
                });
            }
            EngineCommand::PlayNext(track) => {
                let controller = self.controller.clone();
                self.rt.spawn(async move {
                    let _ = controller.play_next(track).await;
                });
            }
            EngineCommand::PlayLast(track) => {
                let controller = self.controller.clone();
                self.rt.spawn(async move {
                    let _ = controller.play_last(track).await;
                });
            }
            EngineCommand::NextTrack => {
                let controller = self.controller.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    match controller.next().await {
                        Ok(()) => {
                            if let Some(t) = controller.snapshot().await.current_track {
                                let _ = controller.expand_autoplay_queue(&t.id).await;
                            }
                        }
                        Err(e) => {
                            let _ = tx
                                .send(LiveUpdate::StatusBanner(format!("Next track failed: {e}")));
                        }
                    }
                });
            }
            EngineCommand::PreviousTrack => {
                let controller = self.controller.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    if let Err(e) = controller.previous().await {
                        let _ = tx.send(LiveUpdate::StatusBanner(format!(
                            "Previous track failed: {e}"
                        )));
                    }
                });
            }
            EngineCommand::JumpQueue(index) => {
                let controller = self.controller.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    match controller.jump_to_queue_index(index).await {
                        Ok(()) => {
                            if let Some(t) = controller.snapshot().await.current_track {
                                let _ = controller.expand_autoplay_queue(&t.id).await;
                            }
                        }
                        Err(e) => {
                            let _ = tx
                                .send(LiveUpdate::StatusBanner(format!("Queue jump failed: {e}")));
                        }
                    }
                });
            }
            EngineCommand::MoveQueueItem { from, to } => {
                let controller = self.controller.clone();
                self.rt.spawn(async move {
                    let _ = controller.reorder_queue(from, to).await;
                });
            }
            EngineCommand::RemoveQueueItem(index) => {
                let controller = self.controller.clone();
                self.rt.spawn(async move {
                    let _ = controller.remove_from_queue(index).await;
                });
            }
            EngineCommand::ClearQueue => {
                let controller = self.controller.clone();
                self.rt.spawn(async move {
                    let _ = controller.clear_queue().await;
                });
            }
            EngineCommand::TogglePlayPause => {
                let controller = self.controller.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    if let Err(e) = controller.toggle_play_pause().await {
                        let _ = tx.send(LiveUpdate::StatusBanner(format!("Playback failed: {e}")));
                    }
                });
            }
            EngineCommand::SeekMs(position_ms) => {
                let controller = self.controller.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    if let Err(e) = controller.seek_ms(position_ms).await {
                        let _ = tx.send(LiveUpdate::StatusBanner(format!("Seek failed: {e}")));
                    }
                });
            }
            EngineCommand::SetVolume(volume) => {
                let controller = self.controller.clone();
                self.rt.spawn(async move {
                    let _ = controller.set_volume(volume.clamp(0.0, 1.0)).await;
                });
            }
            EngineCommand::SetShuffle(enabled) => {
                let controller = self.controller.clone();
                self.rt.spawn(async move {
                    controller.set_shuffle(enabled).await;
                });
            }
            EngineCommand::SetRepeatMode(mode) => {
                let controller = self.controller.clone();
                self.rt.spawn(async move {
                    controller.set_repeat_mode(mode).await;
                });
            }
            EngineCommand::SetAutoplayRadio(enabled) => {
                let controller = self.controller.clone();
                self.rt.spawn(async move {
                    controller.set_autoplay_radio(enabled).await;
                });
            }

            // Auth, Config, Cache & Settings
            EngineCommand::SignIn(cookie_header) => {
                let provider = self.provider.clone();
                let cache = self.cache.clone();
                let tx = self.tx.clone();
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
                            if let Some(inner) = provider.inner_client()
                                && let Ok(profile) =
                                    AccountProfileProbe::probe_and_update(inner.as_ref()).await
                            {
                                let _ = tx.send(LiveUpdate::AccountProfileUpdated(Some(profile)));
                            }
                            let status = platform_auth_status(provider.current_session().as_ref());
                            let _ = tx.send(LiveUpdate::AuthStatus(status));
                            let _ = tx.send(LiveUpdate::AuthUpdated {
                                signed_in: true,
                                banner: "Signed in securely (stored in OS Keystore)".into(),
                            });
                            let snap =
                                refresh_library_snapshot(provider.as_ref(), cache.as_ref()).await;
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
            EngineCommand::SignOut => {
                let provider = self.provider.clone();
                let cache = self.cache.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let _ = provider.sync_session(None).await;
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

            // Optimistic Mutations
            EngineCommand::SetLike { video_id, action } => {
                self.enqueue_mutation(MutationCommand::SetLike { video_id, action });
            }
            EngineCommand::EditPlaylist { playlist_id, op } => {
                self.enqueue_mutation(MutationCommand::EditPlaylist { playlist_id, op });
            }
            EngineCommand::ReportAtr(report) => {
                self.enqueue_mutation(MutationCommand::ReportAtr(report));
            }

            // Offline Vault (single unified facade)
            EngineCommand::EnqueueDownload(track) => {
                let vault = self.vault.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    if let Err(e) = vault.enqueue_track(track).await {
                        let _ = tx.send(LiveUpdate::StatusBanner(format!("Download failed: {e}")));
                    }
                });
            }
            EngineCommand::PauseDownload(video_id) => {
                let vault = self.vault.clone();
                self.rt.spawn(async move {
                    let _ = vault.pause_track(&video_id).await;
                });
            }
            EngineCommand::ResumeDownload(video_id) => {
                let vault = self.vault.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    if let Err(e) = vault.resume_track(&video_id).await {
                        let _ = tx.send(LiveUpdate::StatusBanner(format!("Resume failed: {e}")));
                    }
                });
            }
            EngineCommand::CancelDownload(video_id) => {
                let vault = self.vault.clone();
                self.rt.spawn(async move {
                    let _ = vault.cancel_track(&video_id).await;
                });
            }
            EngineCommand::DeleteDownload(video_id) => {
                let vault = self.vault.clone();
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
                let vault = self.vault.clone();
                let tx = self.tx.clone();
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
                        let _ = tx.send(LiveUpdate::StatusBanner(format!("Pin failed: {e}")));
                    } else if let Ok(list) = vault.list_pinned_collections() {
                        let _ = tx.send(LiveUpdate::PinnedCollectionsLoaded(list));
                    }
                });
            }
            EngineCommand::UnpinCollection(collection_id) => {
                let vault = self.vault.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let _ = vault.unpin_collection(&collection_id);
                    if let Ok(list) = vault.list_pinned_collections() {
                        let _ = tx.send(LiveUpdate::PinnedCollectionsLoaded(list));
                    }
                });
            }
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

fn engine() -> Result<&'static AppEngine, &'static str> {
    match APP_ENGINE.get_or_init(AppEngine::boot) {
        Ok(eng) => Ok(eng),
        Err(err) => Err(err.as_str()),
    }
}

/// Drains pending updates from the global background [`AppEngine`].
pub fn try_drain_updates() -> Vec<LiveUpdate> {
    match engine() {
        Ok(eng) => eng.try_drain_updates(),
        Err(_) => Vec::new(),
    }
}

/// Dispatches an [`EngineCommand`] to the global background [`AppEngine`].
pub fn dispatch(cmd: EngineCommand) {
    match engine() {
        Ok(eng) => eng.dispatch(cmd),
        Err(err) => match cmd {
            EngineCommand::LoadHome => crate::store::set_home_init_error(err.to_owned()),
            EngineCommand::Search { query, .. } => {
                crate::store::set_search_init_error(query, err.to_owned())
            }
            _ => {}
        },
    }
}

// ── Persisted Settings & Library Snapshot Helpers (tested directly in integration suites) ──

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
