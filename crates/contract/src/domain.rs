//! Frozen domain models shared between core, platform bridges, and NSH UI.

use serde::{Deserialize, Serialize};

pub type TrackId = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistRef {
    pub id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumRef {
    pub id: Option<String>,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub album: Option<AlbumRef>,
    pub duration_ms: u64,
    pub artwork_url: Option<String>,
    pub is_explicit: bool,
    pub is_liked: bool,
    pub is_video: bool,
}

impl Track {
    pub fn artist_display(&self) -> String {
        if self.artists.is_empty() {
            "Unknown Artist".to_owned()
        } else {
            self.artists
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    pub fn fallback_monogram(&self) -> String {
        self.title
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "♪".to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub artwork_url: Option<String>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResults {
    pub query: String,
    pub tracks: Vec<Track>,
    pub albums: Vec<Playlist>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeSection {
    pub id: String,
    pub title: String,
    pub items: Vec<HomeItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HomeItem {
    Track(Track),
    Collection {
        browse_id: String,
        title: String,
        subtitle: String,
        artwork_url: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioCodec {
    Opus,
    Aac,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamInfo {
    pub video_id: TrackId,
    pub url: String,
    pub itag: u32,
    pub mime_type: String,
    pub codec: AudioCodec,
    pub bitrate_kbps: u32,
    pub content_length: u64,
    pub duration_ms: u64,
    pub loudness_db: Option<f32>,
    pub resolved_by_client: String,
    pub expires_at_unix_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RepeatMode {
    #[default]
    None,
    One,
    All,
}

impl RepeatMode {
    pub fn cycle(self) -> Self {
        match self {
            RepeatMode::None => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RepeatMode::None => "Repeat Off",
            RepeatMode::All => "Repeat All",
            RepeatMode::One => "Repeat One",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct QueueState {
    pub items: Vec<Track>,
    pub current_index: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub autoplay_radio: bool,
    pub resume_position_ms: u64,
}

/// Non-repeating shuffle order preserving original queue order.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ShuffleDeck {
    /// Original unshuffled indices into `QueueState::items` in shuffled playback order.
    pub order: Vec<usize>,
    /// Current cursor inside `order`.
    pub cursor: usize,
}

impl QueueState {
    pub fn current_track(&self) -> Option<&Track> {
        self.current_index.and_then(|idx| self.items.get(idx))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LyricSyncKind {
    Word,
    Line,
    Plain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricWord {
    pub start_ms: u64,
    pub duration_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricLine {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub words: Vec<LyricWord>,
    pub is_instrumental: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsDocument {
    pub video_id: TrackId,
    pub provider: String,
    pub sync_kind: LyricSyncKind,
    pub lines: Vec<LyricLine>,
}

impl LyricsDocument {
    /// Returns the index of the active lyric line for `position_ms`.
    pub fn active_line_index(&self, position_ms: u64) -> Option<usize> {
        if self.lines.is_empty() || self.sync_kind == LyricSyncKind::Plain {
            return None;
        }
        let mut active = None;
        for (i, line) in self.lines.iter().enumerate() {
            if position_ms >= line.start_ms {
                active = Some(i);
            } else {
                break;
            }
        }
        active
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SponsorSegment {
    pub start_secs: f64,
    pub end_secs: f64,
    pub video_duration_secs: f64,
}

impl SponsorSegment {
    pub fn fits_duration(&self, track_duration_secs: f64) -> bool {
        self.video_duration_secs <= 0.0
            || (self.video_duration_secs - track_duration_secs).abs() < 3.0
    }

    pub fn contains_ms(&self, position_ms: u64) -> bool {
        let secs = position_ms as f64 / 1000.0;
        secs >= self.start_secs && secs < self.end_secs - 0.15
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthSession {
    pub cookie_header: String,
    pub auth_user: u32,
    pub page_id: Option<String>,
    pub visitor_data: Option<String>,
    pub account_name: Option<String>,
    pub is_premium: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CacheStats {
    pub audio_chunk_bytes: u64,
    pub audio_chunk_count: u64,
    pub artwork_bytes: u64,
    pub metadata_entries: u64,
    pub max_audio_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub track: Track,
    pub played_at_unix_secs: u64,
}

pub fn format_duration_ms(ms: u64) -> String {
    let total_secs = ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{mins}:{secs:02}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Paused,
    /// Download finished on disk; SHA-256 integrity verification and metadata tagging in progress.
    Tagging,
    /// Atomically committed and immediately playable offline with zero network I/O.
    Completed,
    Failed,
    Cancelled,
}

impl DownloadStatus {
    /// Strict transition rules enforcing `Queued -> Downloading -> Tagging -> Completed`.
    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        matches!(
            (self, next),
            (
                Self::Queued,
                Self::Downloading | Self::Paused | Self::Cancelled | Self::Failed
            ) | (
                Self::Downloading,
                Self::Tagging | Self::Paused | Self::Failed | Self::Cancelled
            ) | (
                Self::Paused,
                Self::Queued | Self::Downloading | Self::Cancelled
            ) | (Self::Tagging, Self::Completed | Self::Failed)
                | (Self::Failed, Self::Queued | Self::Cancelled)
                | (Self::Cancelled, Self::Queued)
        )
    }

    pub fn transition(self, next: Self) -> Result<Self, String> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(format!("invalid download transition: {self:?} -> {next:?}"))
        }
    }

    pub fn is_available(self) -> bool {
        self == Self::Completed
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadRecord {
    pub video_id: TrackId,
    pub track: Track,
    pub status: DownloadStatus,
    pub itag: u32,
    pub codec: AudioCodec,
    pub mime_type: String,
    /// Relative filename inside downloads root (e.g. `dQw4w9WgXcQ_140.m4a`). Never an absolute path.
    pub relative_path: Option<String>,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
    pub sha256_hex: Option<String>,
    pub tags_embedded: bool,
    pub created_at_unix_secs: u64,
    pub updated_at_unix_secs: u64,
    pub completed_at_unix_secs: Option<u64>,
    /// Redacted human-readable failure reason (must never contain signed URLs or query params).
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OfflineStats {
    pub completed_count: u64,
    pub active_count: u64,
    pub total_bytes: u64,
    pub max_bytes: u64,
}
