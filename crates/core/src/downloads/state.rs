//! First-class Offline Vault download state machine:
//! `Queued -> Downloading -> Verified -> Available` (plus `Paused`, `Failed`, `Cancelled`).

use resonance_contract::DownloadStatus;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VaultTrackState {
    Queued,
    Downloading,
    Paused,
    /// Download finished on disk and SHA-256 integrity + metadata tags verified.
    Verified,
    /// Atomically committed and immediately playable offline with zero network I/O.
    Available,
    Failed,
    Cancelled,
}

impl VaultTrackState {
    /// Strict transition rules enforcing `Queued -> Downloading -> Verified -> Available`.
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
                Self::Verified | Self::Paused | Self::Failed | Self::Cancelled
            ) | (
                Self::Paused,
                Self::Queued | Self::Downloading | Self::Cancelled
            ) | (Self::Verified, Self::Available | Self::Failed)
                | (Self::Failed, Self::Queued | Self::Cancelled)
                | (Self::Cancelled, Self::Queued)
        )
    }

    pub fn transition(self, next: Self) -> Result<Self, String> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(format!(
                "invalid offline vault transition: {self:?} -> {next:?}"
            ))
        }
    }

    pub fn from_download_status(status: DownloadStatus, has_verified_sha: bool) -> Self {
        match status {
            DownloadStatus::Queued => Self::Queued,
            DownloadStatus::Downloading => Self::Downloading,
            DownloadStatus::Paused => Self::Paused,
            DownloadStatus::Tagging => Self::Verified,
            DownloadStatus::Completed => {
                if has_verified_sha {
                    Self::Available
                } else {
                    Self::Verified
                }
            }
            DownloadStatus::Failed => Self::Failed,
            DownloadStatus::Cancelled => Self::Cancelled,
        }
    }

    pub fn to_download_status(self) -> DownloadStatus {
        match self {
            Self::Queued => DownloadStatus::Queued,
            Self::Downloading => DownloadStatus::Downloading,
            Self::Paused => DownloadStatus::Paused,
            Self::Verified => DownloadStatus::Tagging,
            Self::Available => DownloadStatus::Completed,
            Self::Failed => DownloadStatus::Failed,
            Self::Cancelled => DownloadStatus::Cancelled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_queued_downloading_verified_available_fsm() {
        let s = VaultTrackState::Queued;
        let s = s.transition(VaultTrackState::Downloading).unwrap();
        let s = s.transition(VaultTrackState::Verified).unwrap();
        let s = s.transition(VaultTrackState::Available).unwrap();
        assert_eq!(s, VaultTrackState::Available);

        // Skipping Downloading -> Available directly without Verified is rejected
        assert!(
            VaultTrackState::Queued
                .transition(VaultTrackState::Available)
                .is_err()
        );
        assert!(
            VaultTrackState::Downloading
                .transition(VaultTrackState::Available)
                .is_err()
        );
    }
}
