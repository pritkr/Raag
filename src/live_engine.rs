//! Live Mode Background Engine Bridge (Dedicated Tokio Runtime off the NSH UI Thread).
//!
//! When Resonance runs in live mode (`--fixture` NOT enabled), this module owns the real
//! production `InnerTubeClient`, `QuickJsSolver`, `SqliteCacheStore`, `ChunkCache`,
//! `LoopbackProxy`, `CascadeLyricsProvider`, `HttpSponsorBlockProvider`, and
//! `PlaybackController` on a dedicated background Tokio runtime and streams typed updates
//! back to the NSH reactive store via a non-blocking channel.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use crossbeam_channel::{Receiver, Sender, unbounded};
use resonance_contract::{
    AppEvent, AuthBackendStatus, AuthSession, CacheStats, CacheStore, DownloadRecord, HomeSection,
    LikeAction, LyricsDocument, LyricsProvider, MusicProvider, MutationCommand, OfflineStore,
    PlayerSnapshot, Playlist, PlaylistEditOp, PoTokenDiagnostics, PoTokenProvider, SearchResults,
    Track, TrackId, WatchtimeReport,
};
use resonance_core::cache::{
    ArtworkCache, ChunkCache, DEFAULT_ARTWORK_DISK_CAP, DEFAULT_MAX_AUDIO_CACHE_BYTES,
    SqliteCacheStore,
};
use resonance_core::downloads::OfflineVault;
use resonance_core::innertube::auth::redact_secrets;
use resonance_core::innertube::{InnerTubeClient, MutationWorker};
use resonance_core::lyrics::CascadeLyricsProvider;
use resonance_core::offline::DownloadManager;
use resonance_core::playback::PlaybackController;
use resonance_core::potoken::{
    CachingPoTokenProvider, ChainedPoTokenProvider, PlatformAttestationPoTokenProvider,
    QuickJsVmAttestationBridge, SidecarPoTokenProvider,
};
use resonance_core::proxy::LoopbackProxy;
use resonance_core::solver::QuickJsSolver;
use resonance_core::sponsorblock::HttpSponsorBlockProvider;
use tokio::runtime::Runtime;

use crate::platform::{
    create_platform_audio_backend, create_platform_auth_store, platform_auth_status,
};

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
    SearchLoaded {
        query: String,
        result: Result<SearchResults, String>,
        plays: HashMap<String, String>,
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
    PoTokenDiagnostics(PoTokenDiagnostics),
    MutationFailed {
        command: MutationCommand,
        reason: String,
    },
    DownloadsLoaded(Vec<DownloadRecord>),
    DownloadUpdated(DownloadRecord),
    DownloadRemoved(TrackId),
    StatusBanner(String),
}

struct LiveRuntime {
    rt: Runtime,
    provider: Arc<InnerTubeClient>,
    cache: Arc<SqliteCacheStore>,
    chunks: Arc<ChunkCache>,
    lyrics: Arc<CascadeLyricsProvider>,
    downloads: Arc<DownloadManager>,
    #[allow(dead_code)]
    vault: Arc<OfflineVault>,
    controller: Arc<PlaybackController>,
    mutations: MutationWorker,
    tx: Sender<LiveUpdate>,
    rx: Receiver<LiveUpdate>,
}

static LIVE_RUNTIME: OnceLock<Result<LiveRuntime, String>> = OnceLock::new();

fn data_dir() -> PathBuf {
    std::env::var_os("RESONANCE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("resonance-ytmusic-live"))
}

fn init_runtime() -> Result<LiveRuntime, String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("resonance-core")
        .build()
        .map_err(|e| format!("failed to build Tokio runtime: {e}"))?;

    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("failed to create data dir: {e}"))?;
    #[cfg(target_os = "android")]
    let _ = crate::platform::android::ensure_downloads_dir(&dir);
    #[cfg(not(target_os = "android"))]
    let _ = crate::platform::desktop::ensure_downloads_dir(&dir);

    let cache = Arc::new(
        SqliteCacheStore::open(&dir).map_err(|e| format!("SQLite cache open failed: {e}"))?,
    );
    let chunks = Arc::new(
        ChunkCache::new(
            &dir.join("chunks"),
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
    let solver = Arc::new(QuickJsSolver::new(&dir).with_sqlite_cache(cache.clone()));
    let provider = Arc::new(
        InnerTubeClient::new(cache.clone(), auth, solver)
            .map_err(|e| format!("InnerTubeClient init failed: {e}"))?,
    );
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

    let audio = create_platform_audio_backend();
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
            provider.clone(),
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
                        signed_in: false,
                        ..
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
    let stats = cache.stats().unwrap_or_default();
    let initial_downloads = cache.list_downloads().unwrap_or_default();
    let current_session = provider.current_session();
    let signed_in = current_session.is_some();
    let auth_status = platform_auth_status(current_session.as_ref());
    let proxy_base = format!("http://{}", proxy.addr());
    crate::store::set_proxy_base_url(&proxy_base);
    let _ = tx.send(LiveUpdate::Ready {
        proxy_base_url: proxy_base,
        snapshot: initial_snap,
        cache_stats: stats,
        signed_in,
    });
    let _ = tx.send(LiveUpdate::AuthStatus(auth_status));
    let _ = tx.send(LiveUpdate::PoTokenDiagnostics(
        provider.po_token_diagnostics(),
    ));
    let _ = tx.send(LiveUpdate::DownloadsLoaded(initial_downloads));

    Ok(LiveRuntime {
        rt,
        provider,
        cache,
        chunks,
        lyrics,
        downloads,
        vault,
        controller,
        mutations,
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
    let provider = rt.provider.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let res = provider.home().await.map_err(|e| e.to_string());
        let _ = tx.send(LiveUpdate::HomeLoaded(res));
    });
}

pub fn dispatch_load_explore() {
    let Ok(rt) = runtime() else { return };
    let provider = rt.provider.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let res = provider.explore().await.map_err(|e| e.to_string());
        let _ = tx.send(LiveUpdate::ExploreLoaded(res));
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

pub fn dispatch_sign_in(cookie_header: String) {
    let Ok(rt) = runtime() else { return };
    let provider = rt.provider.clone();
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
                let status = platform_auth_status(provider.current_session().as_ref());
                let _ = tx.send(LiveUpdate::AuthStatus(status));
                let _ = tx.send(LiveUpdate::AuthUpdated {
                    signed_in: true,
                    banner: "Signed in securely (stored in OS Keystore)".into(),
                });
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
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let _ = provider.sign_out();
        let status = platform_auth_status(provider.current_session().as_ref());
        let _ = tx.send(LiveUpdate::AuthStatus(status));
        let _ = tx.send(LiveUpdate::AuthUpdated {
            signed_in: false,
            banner: "Signed out — running in anonymous mode".into(),
        });
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
