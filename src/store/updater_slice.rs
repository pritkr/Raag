//! Reactive store slice for LT-2 Native NSH Auto-Updater (`UpdaterSlice`).

use resonance_contract::{UpdateChannel, UpdateInfo, UpdatePhase};

#[derive(Debug, Clone)]
pub struct UpdaterSlice {
    pub channel: UpdateChannel,
    pub auto_check: bool,
    pub skipped_version: Option<String>,
    pub phase: UpdatePhase,
    pub current_version: &'static str,
    pub last_check_unix_secs: Option<u64>,
    pub banner_dismissed: bool,
}

impl Default for UpdaterSlice {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdaterSlice {
    pub fn new() -> Self {
        Self {
            channel: UpdateChannel::Stable,
            auto_check: true,
            skipped_version: None,
            phase: UpdatePhase::Idle,
            current_version: env!("CARGO_PKG_VERSION"),
            last_check_unix_secs: None,
            banner_dismissed: false,
        }
    }

    /// Single-flight guard: returns `true` only if a new check may begin.
    pub fn begin_check(&mut self) -> bool {
        if matches!(
            self.phase,
            UpdatePhase::Checking | UpdatePhase::Downloading { .. }
        ) {
            return false;
        }
        self.phase = UpdatePhase::Checking;
        true
    }

    pub fn mark_up_to_date(&mut self, checked_at_unix_secs: u64) {
        self.last_check_unix_secs = Some(checked_at_unix_secs);
        self.phase = UpdatePhase::UpToDate {
            checked_at_unix_secs,
        };
    }

    pub fn mark_available(&mut self, info: UpdateInfo, checked_at_unix_secs: u64) {
        self.last_check_unix_secs = Some(checked_at_unix_secs);
        self.banner_dismissed = false;
        self.phase = UpdatePhase::Available(info);
    }

    /// Single-flight guard: returns `true` only if a new download may begin.
    pub fn begin_download(&mut self) -> Option<UpdateInfo> {
        let UpdatePhase::Available(info) = &self.phase else {
            return None;
        };
        if !info.can_install_in_place {
            return None;
        }
        let cloned = info.clone();
        self.phase = UpdatePhase::Downloading {
            info: cloned.clone(),
            received_bytes: 0,
            total_bytes: None,
        };
        Some(cloned)
    }

    pub fn update_download_progress(&mut self, received_bytes: u64, total_bytes: Option<u64>) {
        if let UpdatePhase::Downloading {
            received_bytes: r,
            total_bytes: t,
            ..
        } = &mut self.phase
        {
            *r = received_bytes;
            *t = total_bytes;
        }
    }

    pub fn mark_ready_to_install(&mut self, install_on_quit_scheduled: bool) {
        let info = match &self.phase {
            UpdatePhase::Downloading { info, .. }
            | UpdatePhase::Available(info)
            | UpdatePhase::ReadyToInstall { info, .. } => info.clone(),
            _ => return,
        };
        self.phase = UpdatePhase::ReadyToInstall {
            info,
            install_on_quit_scheduled,
        };
    }

    pub fn mark_error(&mut self, msg: impl Into<String>) {
        self.phase = UpdatePhase::Error(msg.into());
    }

    pub fn skip_available_version(&mut self) {
        if let UpdatePhase::Available(info) = &self.phase {
            self.skipped_version = Some(info.version.clone());
            self.banner_dismissed = true;
            self.phase = UpdatePhase::Idle;
        }
    }

    pub fn set_channel(&mut self, channel: UpdateChannel) {
        if self.channel != channel {
            self.channel = channel;
            self.phase = UpdatePhase::Idle;
        }
    }

    pub fn set_auto_check(&mut self, enabled: bool) {
        self.auto_check = enabled;
    }

    pub fn should_show_top_banner(&self) -> Option<&UpdateInfo> {
        if self.banner_dismissed {
            return None;
        }
        if let UpdatePhase::Available(info) = &self.phase
            && self.skipped_version.as_deref() != Some(info.version.as_str())
        {
            return Some(info);
        }
        None
    }
}
