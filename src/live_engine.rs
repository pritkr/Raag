//! Live Mode Background Engine Bridge (Dedicated Tokio Runtime off the NSH UI Thread).
//!
//! When Resonance runs in live mode (`--fixture` NOT enabled), this module owns the real
//! production `InnerTubeClient`, `QuickJsSolver`, `SqliteCacheStore`, `ChunkCache`,
//! `LoopbackProxy`, `CascadeLyricsProvider`, `HttpSponsorBlockProvider`, and
//! `PlaybackController` on a dedicated background Tokio runtime and streams typed updates
//! back to the NSH reactive store via a non-blocking channel.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use crossbeam_channel::{Receiver, Sender, unbounded};
use resonance_contract::{
    AppEvent, AuthSession, CacheStats, CacheStore, HomeSection, LikeAction, LyricsDocument,
    LyricsProvider, MusicProvider, MutationCommand, PlayerSnapshot, Playlist, PlaylistEditOp,
    SearchResults, Track, TrackId, WatchtimeReport,
};
use resonance_core::cache::{ChunkCache, DEFAULT_MAX_AUDIO_CACHE_BYTES, SqliteCacheStore};
use resonance_core::innertube::{FileAuthStore, InnerTubeClient, MutationWorker};
use resonance_core::lyrics::CascadeLyricsProvider;
use resonance_core::playback::PlaybackController;
use resonance_core::potoken::{CachingPoTokenProvider, SidecarPoTokenProvider};
use resonance_core::proxy::LoopbackProxy;
use resonance_core::solver::QuickJsSolver;
use resonance_core::sponsorblock::HttpSponsorBlockProvider;
use tokio::runtime::Runtime;

use crate::platform::create_platform_audio_backend;

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
    SearchLoaded {
        query: String,
        result: Result<SearchResults, String>,
    },
    CollectionLoaded(Result<Playlist, String>),
    PlaybackUpdated(PlayerSnapshot),
    LyricsLoaded(Option<LyricsDocument>),
    CacheStatsUpdated(CacheStats),
    AuthUpdated {
        signed_in: bool,
        banner: String,
    },
    MutationFailed {
        command: MutationCommand,
        reason: String,
    },
    StatusBanner(String),
}

struct LiveRuntime {
    rt: Runtime,
    provider: Arc<InnerTubeClient>,
    cache: Arc<SqliteCacheStore>,
    chunks: Arc<ChunkCache>,
    lyrics: Arc<CascadeLyricsProvider>,
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

    let cache = Arc::new(
        SqliteCacheStore::open(&dir.join("resonance.db"))
            .map_err(|e| format!("SQLite cache open failed: {e}"))?,
    );
    let chunks = Arc::new(
        ChunkCache::new(
            &dir.join("chunks"),
            cache.clone(),
            DEFAULT_MAX_AUDIO_CACHE_BYTES,
        )
        .map_err(|e| format!("ChunkCache init failed: {e}"))?,
    );
    let auth = Arc::new(FileAuthStore::new(&dir));
    let solver = Arc::new(QuickJsSolver::new(&dir));
    let provider = Arc::new(
        InnerTubeClient::new(cache.clone(), auth, solver)
            .map_err(|e| format!("InnerTubeClient init failed: {e}"))?,
    );
    if let Ok(pot_url) = std::env::var("RESONANCE_POTOKEN_URL")
        && !pot_url.trim().is_empty()
    {
        match SidecarPoTokenProvider::new(pot_url.trim()) {
            Ok(sidecar) => {
                let cached = CachingPoTokenProvider::new(sidecar).into_arc();
                provider.set_po_token_provider(Some(cached));
            }
            Err(e) => {
                log::warn!("ignoring invalid RESONANCE_POTOKEN_URL: {e}");
            }
        }
    }
    let proxy = rt.block_on(async {
        LoopbackProxy::start(provider.clone(), chunks.clone())
            .await
            .map(Arc::new)
            .map_err(|e| format!("LoopbackProxy start failed: {e}"))
    })?;

    let audio = create_platform_audio_backend();
    let lyrics = Arc::new(CascadeLyricsProvider::new(provider.clone(), cache.clone()));
    let sponsorblock = Arc::new(HttpSponsorBlockProvider::new());
    let bus = resonance_contract::EventBus::new(128);
    let mutations = {
        let _guard = rt.enter();
        MutationWorker::spawn(provider.clone(), bus.clone())
    };
    let controller = Arc::new(PlaybackController::new(
        provider.clone(),
        audio,
        cache.clone(),
        proxy.clone(),
        lyrics.clone(),
        sponsorblock,
        bus.clone(),
    ));

    let (tx, rx) = unbounded();
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
                    _ => {}
                }
            }
        });
    }
    let initial_snap = rt.block_on(controller.snapshot());
    let stats = cache.stats().unwrap_or_default();
    let signed_in = provider.current_session().is_some();
    let proxy_base = format!("http://{}", proxy.addr());
    crate::store::set_proxy_base_url(&proxy_base);
    let _ = tx.send(LiveUpdate::Ready {
        proxy_base_url: proxy_base,
        snapshot: initial_snap,
        cache_stats: stats,
        signed_in,
    });

    Ok(LiveRuntime {
        rt,
        provider,
        cache,
        chunks,
        lyrics,
        controller,
        mutations,
        tx,
        rx,
    })
}

fn runtime() -> Option<&'static LiveRuntime> {
    LIVE_RUNTIME.get_or_init(init_runtime).as_ref().ok()
}

pub fn try_drain_updates() -> Vec<LiveUpdate> {
    let Some(rt) = runtime() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    while let Ok(ev) = rt.rx.try_recv() {
        out.push(ev);
    }
    out
}

pub fn dispatch_load_home() {
    let Some(rt) = runtime() else { return };
    let provider = rt.provider.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let res = provider.home().await.map_err(|e| e.to_string());
        let _ = tx.send(LiveUpdate::HomeLoaded(res));
    });
}

pub fn dispatch_search(query: String) {
    let Some(rt) = runtime() else { return };
    let provider = rt.provider.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let res = provider.search(&query).await.map_err(|e| e.to_string());
        let _ = tx.send(LiveUpdate::SearchLoaded { query, result: res });
    });
}

pub fn dispatch_browse_collection(browse_id: String) {
    let Some(rt) = runtime() else { return };
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
    let Some(rt) = runtime() else { return };
    let controller = rt.controller.clone();
    let lyrics = rt.lyrics.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let track_for_lyrics = track.clone();
        match controller.play_track(track).await {
            Ok(()) => {
                let snap = controller.snapshot().await;
                let _ = tx.send(LiveUpdate::PlaybackUpdated(snap));
            }
            Err(e) => {
                let _ = tx.send(LiveUpdate::StatusBanner(format!("Playback failed: {e}")));
            }
        }
        let lyr = lyrics.fetch_lyrics(&track_for_lyrics).await.ok().flatten();
        let _ = tx.send(LiveUpdate::LyricsLoaded(lyr));
    });
}

pub fn dispatch_sign_in(cookie_header: String) {
    let Some(rt) = runtime() else { return };
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
                let _ = tx.send(LiveUpdate::AuthUpdated {
                    signed_in: true,
                    banner: "Signed in securely (stored in OS Keystore)".into(),
                });
            }
            Err(e) => {
                let _ = tx.send(LiveUpdate::StatusBanner(format!("Sign-in failed: {e}")));
            }
        }
    });
}

pub fn dispatch_sign_out() {
    let Some(rt) = runtime() else { return };
    let provider = rt.provider.clone();
    let tx = rt.tx.clone();
    rt.rt.spawn(async move {
        let _ = provider.sign_out();
        let _ = tx.send(LiveUpdate::AuthUpdated {
            signed_in: false,
            banner: "Signed out — running in anonymous mode".into(),
        });
    });
}

pub fn dispatch_clear_caches() {
    let Some(rt) = runtime() else { return };
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
    let Some(rt) = runtime() else { return };
    let cmd = MutationCommand::SetLike { video_id, action };
    if let Err(e) = rt.mutations.enqueue(cmd.clone()) {
        let _ = rt.tx.send(LiveUpdate::MutationFailed {
            command: cmd,
            reason: e.to_string(),
        });
    }
}

pub fn dispatch_edit_playlist(playlist_id: String, op: PlaylistEditOp) {
    let Some(rt) = runtime() else { return };
    let cmd = MutationCommand::EditPlaylist { playlist_id, op };
    if let Err(e) = rt.mutations.enqueue(cmd.clone()) {
        let _ = rt.tx.send(LiveUpdate::MutationFailed {
            command: cmd,
            reason: e.to_string(),
        });
    }
}

pub fn dispatch_report_atr(report: WatchtimeReport) {
    let Some(rt) = runtime() else { return };
    let cmd = MutationCommand::ReportAtr(report);
    if let Err(e) = rt.mutations.enqueue(cmd.clone()) {
        let _ = rt.tx.send(LiveUpdate::MutationFailed {
            command: cmd,
            reason: e.to_string(),
        });
    }
}
