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

mod boot;
mod commands;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crossbeam_channel::{Receiver, Sender};
use resonance_contract::{
    AccountProfile, AuthBackendStatus, CacheStats, DownloadRecord, ExplorePage, HomeSection,
    LibraryCollectionRef, LibrarySnapshot, LikeAction, LyricsDocument, MutationCommand,
    PlayerSnapshot, Playlist, PlaylistEditOp, PoTokenDiagnostics, RemoteConfigStatus, RepeatMode,
    SearchCategory, SearchResults, SettingsV1, Track, TrackId, WatchtimeReport,
};
pub use resonance_core::cache::{
    load_cached_library_snapshot, load_persisted_settings, save_persisted_settings,
};
use resonance_core::cache::{ChunkCache, SqliteCacheStore};
use resonance_core::innertube::{MutationWorker, RemoteConfigSyncService};
use resonance_core::offline::{OfflineVault, PinnedCollection, PinnedCollectionKind};
use resonance_core::playback::PlaybackController;
pub use resonance_core::providers::refresh_library_snapshot;
use resonance_core::providers::HybridMusicProvider;
use resonance_core::solver::PlayerScriptFetcher;
use tokio::runtime::Runtime;

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
    pub(super) rt: Runtime,
    pub(super) provider: Arc<HybridMusicProvider>,
    pub(super) _player_fetcher: Arc<PlayerScriptFetcher>,
    pub(super) remote_config: Arc<RemoteConfigSyncService>,
    pub(super) cache: Arc<SqliteCacheStore>,
    pub(super) chunks: Arc<ChunkCache>,
    pub(super) vault: Arc<OfflineVault>,
    pub(super) controller: Arc<PlaybackController>,
    pub(super) mutations: MutationWorker,
    pub(super) settings_tx: tokio::sync::mpsc::UnboundedSender<(SettingsV1, bool)>,
    pub(super) tx: Sender<LiveUpdate>,
    pub(super) rx: Receiver<LiveUpdate>,
}

static APP_ENGINE: OnceLock<Result<AppEngine, String>> = OnceLock::new();

impl AppEngine {
    /// Drains all pending [`LiveUpdate`]s without blocking.
    pub fn try_drain_updates(&self) -> Vec<LiveUpdate> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            out.push(ev);
        }
        out
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
