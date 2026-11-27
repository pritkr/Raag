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
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex, OnceLock};

use crossbeam_channel::{Receiver, Sender};
use resonance_contract::{
    AccountProfile, AlbumDetail, ArtistDetail, AudioDspCapability, AuthBackendStatus, CacheStats,
    CastDevice, CastSessionState, DownloadRecord, EventBus, ExplorePage, HomeSection,
    LibraryCollectionRef, LibrarySnapshot, LikeAction, LocalFolderRoot, LocalScanProgress,
    LocalTrackRecord, LyricsDocument, MoodCategory, MutationCommand, MutationReceipt,
    PlayerSnapshot, Playlist, PlaylistEditOp, PlaylistPrivacy, PoTokenDiagnostics,
    QueuePermission, RemoteConfigStatus, RepeatMode, SearchCategory, SearchResults, SettingsV1,
    SleepTimerState, Track, TrackId, UpdateChannel, UpdateInfo, UpdatePhase, WatchtimeReport,
};
use resonance_core::audio_dsp::DspConfig;
pub use resonance_core::cache::{
    load_cached_library_snapshot, load_persisted_settings, save_persisted_settings,
};
use resonance_core::cache::{ArtworkCache, ChunkCache, SqliteCacheStore};
use resonance_core::casting::{CastDiscoveryService, CastManager};
use resonance_core::innertube::{MutationWorker, RemoteConfigSyncService};
use resonance_core::local_media::LocalMediaStore;
use resonance_core::offline::{OfflineVault, PinnedCollection, PinnedCollectionKind};
use resonance_core::playback::PlaybackController;
pub use resonance_core::providers::refresh_library_snapshot;
use resonance_core::providers::HybridMusicProvider;
use resonance_core::rooms::RoomStateSnapshot;
use resonance_core::solver::PlayerScriptFetcher;
use resonance_core::updater::UpdaterService;
use tokio::runtime::Runtime;

/// Internal command channel sent to the active Listen Together room task.
#[derive(Debug, Clone)]
pub(super) enum RoomTaskCmd {
    Leave,
    VoteSkip,
    SendReaction(String),
    KickParticipant(String),
    TransferHost(String),
}

/// Typed commands dispatched from the NSH UI store to [`AppEngine`].
#[derive(Debug, Clone)]
pub enum EngineCommand {
    // Catalog & Search
    LoadHome,
    LoadHomeWithMood {
        mood: Option<String>,
        params: Option<String>,
    },
    LoadExplore,
    LoadMoodPlaylists {
        generation: u64,
        mood: String,
        params: String,
    },
    LoadAlbumEntity {
        generation: u64,
        browse_id: String,
    },
    LoadArtistEntity {
        generation: u64,
        channel_id: String,
    },
    LoadRelatedTracks {
        video_id: String,
    },
    Search {
        generation: u64,
        query: String,
        params: Option<&'static str>,
    },
    SearchSuggestions {
        prefix: String,
    },
    SearchContinuation {
        generation: u64,
        query: String,
        category: SearchCategory,
        continuation: String,
    },
    BrowseCollection {
        generation: u64,
        browse_id: String,
    },

    // Playback, Queue & Audio DSP
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
    ClearUpNext,
    ClearQueue,
    TogglePlayPause,
    SeekMs(u64),
    SetVolume(f32),
    SetShuffle(bool),
    SetRepeatMode(RepeatMode),
    SetAutoplayRadio(bool),
    SetSponsorBlock(bool),
    SetSleepTimer(SleepTimerState),
    SetPlaybackSpeed {
        speed: f32,
        preserve_pitch: bool,
    },
    UpdateDsp(DspConfig),

    // Auth, Config, Cache & Settings
    SignIn(String),
    SignOut,
    StartTvOAuth,
    CancelTvOAuth,
    LaunchAndroidWebViewSignIn,
    RefreshRemoteConfig,
    ClearCaches,
    PersistSettings {
        settings: SettingsV1,
        immediate: bool,
    },
    SetAtrSync(bool),

    // Optimistic Mutations & Playlist CRUD
    SetLike {
        video_id: TrackId,
        action: LikeAction,
    },
    EditPlaylist {
        playlist_id: String,
        op: PlaylistEditOp,
    },
    CreatePlaylist {
        local_temp_id: String,
        title: String,
        description: String,
        privacy: PlaylistPrivacy,
        initial_video_ids: Vec<TrackId>,
    },
    RenamePlaylist {
        playlist_id: String,
        new_title: String,
    },
    DeletePlaylist {
        playlist_id: String,
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

    // Local Media Library
    LoadLocalMedia,
    AddLocalFolder(PathBuf),
    RemoveLocalFolder(String),
    RescanLocalMedia,

    // Listen Together Rooms
    CreateRoom {
        relay_url: String,
        display_name: String,
        queue_permission: QueuePermission,
    },
    JoinRoom {
        relay_url: String,
        room_code: String,
        display_name: String,
    },
    LeaveRoom,
    VoteSkipRoom,
    SendRoomReaction(String),
    KickRoomParticipant(String),
    TransferRoomHost(String),

    // LAN Casting
    StartCastDiscovery,
    StopCastDiscovery,
    ConnectCastDevice(CastDevice),
    DisconnectCast,
    SetCastVolume(f32),

    // Signed Native Auto-Updater
    CheckForUpdates {
        channel: UpdateChannel,
        skipped_version: Option<String>,
    },
    DownloadUpdate(UpdateInfo),
    CancelUpdateDownload,
    InstallUpdateOnQuit,
    RestartToInstallUpdate,
}

/// Typed updates pushed from [`AppEngine`] to the NSH UI thread.
#[derive(Debug, Clone)]
pub enum LiveUpdate {
    Ready {
        proxy_base_url: String,
        snapshot: PlayerSnapshot,
        cache_stats: CacheStats,
        signed_in: bool,
        dsp_capability: AudioDspCapability,
    },
    HomeLoaded(Result<Vec<HomeSection>, String>),
    HomeMoodLoaded {
        mood: Option<String>,
        chips: Vec<MoodCategory>,
        result: Result<Vec<HomeSection>, String>,
    },
    ExplorePageLoaded(Result<ExplorePage, String>),
    MoodPlaylistsLoaded {
        generation: u64,
        mood: String,
        result: Result<Vec<LibraryCollectionRef>, String>,
    },
    AlbumEntityLoaded {
        generation: u64,
        result: Result<AlbumDetail, String>,
    },
    ArtistEntityLoaded {
        generation: u64,
        result: Result<ArtistDetail, String>,
    },
    RelatedTracksLoaded {
        video_id: String,
        tracks: Vec<Track>,
    },
    SearchLoaded {
        generation: u64,
        query: String,
        result: Result<SearchResults, String>,
        plays: HashMap<String, String>,
    },
    SearchSuggestionsLoaded {
        prefix: String,
        suggestions: Vec<String>,
    },
    SearchContinuationLoaded {
        generation: u64,
        query: String,
        result: Result<SearchResults, String>,
    },
    CollectionLoaded {
        generation: u64,
        result: Result<Playlist, String>,
    },
    PlaybackUpdated(PlayerSnapshot),
    LyricsLoaded {
        video_id: TrackId,
        lyrics: Option<LyricsDocument>,
    },
    CacheStatsUpdated(CacheStats),
    AuthUpdated {
        signed_in: bool,
        banner: String,
    },
    AuthStatus(AuthBackendStatus),
    AccountProfileUpdated(Option<AccountProfile>),
    TvOAuthDeviceCode {
        user_code: String,
        verification_url: String,
        expires_in_secs: u64,
    },
    TvOAuthStatus(String),
    TvOAuthCompleted {
        ok: bool,
        message: String,
    },
    RemoteConfigUpdated(RemoteConfigStatus),
    PoTokenDiagnostics(PoTokenDiagnostics),
    MutationCompleted(MutationReceipt),
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
    LocalMediaLoaded {
        roots: Vec<LocalFolderRoot>,
        tracks: Vec<LocalTrackRecord>,
    },
    LocalScanProgressUpdated(LocalScanProgress),
    RoomSnapshotUpdated(Option<RoomStateSnapshot>),
    RoomVoteSkipUpdated {
        votes: usize,
        threshold: usize,
        skipped: bool,
    },
    RoomReactionReceived(String),
    CastDevicesUpdated(Vec<CastDevice>),
    CastSessionUpdated(CastSessionState),
    UpdaterPhaseUpdated(UpdatePhase),
    StatusBanner(String),
}

/// Instantiable background engine coordinating network, storage, proxy, playback, and downloads.
pub struct AppEngine {
    pub(super) rt: Runtime,
    pub(super) provider: Arc<HybridMusicProvider>,
    pub(super) player_fetcher: Arc<PlayerScriptFetcher>,
    pub(super) remote_config: Arc<RemoteConfigSyncService>,
    pub(super) cache: Arc<SqliteCacheStore>,
    pub(super) chunks: Arc<ChunkCache>,
    pub(super) artwork: Arc<ArtworkCache>,
    pub(super) local_media: Option<Arc<LocalMediaStore>>,
    pub(super) proxy_base_url: String,
    pub(super) vault: Arc<OfflineVault>,
    pub(super) controller: Arc<PlaybackController>,
    pub(super) updater: Arc<UpdaterService>,
    pub(super) cast_manager: Arc<CastManager>,
    pub(super) cast_discovery: Arc<Mutex<Option<CastDiscoveryService>>>,
    pub(super) room_tx: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<RoomTaskCmd>>>>,
    #[cfg(target_os = "android")]
    pub(super) android_bridge: Arc<crate::platform::android::AndroidMediaBridge>,
    pub(super) mutations: MutationWorker,
    pub(super) bus: EventBus,
    pub(super) auth_generation: Arc<AtomicU64>,
    pub(super) explicit_sign_out_pending: Arc<AtomicU64>,
    pub(super) tv_oauth_cancel: Arc<Mutex<Option<tokio::sync::watch::Sender<bool>>>>,
    pub(super) atr_enabled: Arc<AtomicBool>,
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
            EngineCommand::LoadHome | EngineCommand::LoadHomeWithMood { .. } => {
                crate::store::set_home_init_error(err.to_owned())
            }
            EngineCommand::Search { query, .. } => {
                crate::store::set_search_init_error(query, err.to_owned())
            }
            _ => {}
        },
    }
}
