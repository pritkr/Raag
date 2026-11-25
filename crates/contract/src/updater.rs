//! Native NSH auto-updater contracts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
}

impl UpdateChannel {
    pub fn nsh_channel_name(self) -> Option<&'static str> {
        match self {
            Self::Stable => None,
            Self::Beta => Some("beta"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "Stable",
            Self::Beta => "Beta",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    pub release_notes: String,
    pub download_url: String,
    pub can_install_in_place: bool,
    pub published_at: Option<String>,
    pub sha256_hex: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UpdatePhase {
    #[default]
    Idle,
    Checking,
    UpToDate {
        checked_at_unix_secs: u64,
    },
    Available(UpdateInfo),
    Downloading {
        info: UpdateInfo,
        received_bytes: u64,
        total_bytes: Option<u64>,
    },
    ReadyToInstall {
        info: UpdateInfo,
        install_on_quit_scheduled: bool,
    },
    Error(String),
}
