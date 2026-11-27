//! Navigation, sidebar/panel chrome, and global status toast slice (`UiSlice`).

use resonance_contract::{SleepTimerState, Track};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SleepTimerMode {
    #[default]
    Off,
    Countdown {
        ends_at_unix_ms: u64,
        duration_secs: u64,
    },
    EndOfTrack,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SleepTimerTickAction {
    None,
    FadeVolume(f32),
    PauseAndRestoreVolume(f32),
}

/// Deterministic sleep timer controller with 3-second gentle fade-out and volume restoration.
#[derive(Debug, Clone, Default)]
pub struct SleepTimerController {
    pub mode: SleepTimerMode,
    pub saved_volume: Option<f32>,
    pub fade_started: bool,
}

impl SleepTimerController {
    pub const FADE_WINDOW_MS: u64 = 3_000;

    pub fn set_countdown(&mut self, duration_secs: u64, now_unix_ms: u64, current_volume: f32) {
        if duration_secs == 0 {
            self.mode = SleepTimerMode::Off;
            self.saved_volume = None;
            self.fade_started = false;
            return;
        }
        if self.saved_volume.is_none() {
            self.saved_volume = Some(current_volume);
        }
        self.fade_started = false;
        self.mode = SleepTimerMode::Countdown {
            ends_at_unix_ms: now_unix_ms.saturating_add(duration_secs.saturating_mul(1000)),
            duration_secs,
        };
    }

    pub fn extend_minutes(&mut self, add_secs: u64, now_unix_ms: u64) {
        if let SleepTimerMode::Countdown {
            ends_at_unix_ms,
            duration_secs,
        } = &mut self.mode
        {
            let base = (*ends_at_unix_ms).max(now_unix_ms);
            *ends_at_unix_ms = base.saturating_add(add_secs.saturating_mul(1000));
            *duration_secs = duration_secs.saturating_add(add_secs);
            self.fade_started = false;
        }
    }

    pub fn set_end_of_track(&mut self, current_volume: f32) {
        if self.saved_volume.is_none() {
            self.saved_volume = Some(current_volume);
        }
        self.fade_started = false;
        self.mode = SleepTimerMode::EndOfTrack;
    }

    /// Cancels any active sleep timer and returns the saved volume if fade had attenuated it.
    pub fn cancel(&mut self) -> Option<f32> {
        let restore = if self.fade_started {
            self.saved_volume
        } else {
            None
        };
        self.mode = SleepTimerMode::Off;
        self.saved_volume = None;
        self.fade_started = false;
        restore
    }

    pub fn tick(&mut self, now_unix_ms: u64) -> SleepTimerTickAction {
        let SleepTimerMode::Countdown {
            ends_at_unix_ms, ..
        } = self.mode
        else {
            return SleepTimerTickAction::None;
        };

        let remaining_ms = ends_at_unix_ms.saturating_sub(now_unix_ms);
        let saved = self.saved_volume.unwrap_or(1.0);

        if remaining_ms == 0 {
            self.mode = SleepTimerMode::Off;
            self.saved_volume = None;
            self.fade_started = false;
            return SleepTimerTickAction::PauseAndRestoreVolume(saved);
        }

        if remaining_ms <= Self::FADE_WINDOW_MS {
            self.fade_started = true;
            let factor = (remaining_ms as f32 / Self::FADE_WINDOW_MS as f32).clamp(0.0, 1.0);
            return SleepTimerTickAction::FadeVolume(saved * factor);
        }

        SleepTimerTickAction::None
    }

    pub fn on_track_ended(&mut self) -> SleepTimerTickAction {
        if self.mode == SleepTimerMode::EndOfTrack {
            let saved = self.saved_volume.unwrap_or(1.0);
            self.mode = SleepTimerMode::Off;
            self.saved_volume = None;
            self.fade_started = false;
            SleepTimerTickAction::PauseAndRestoreVolume(saved)
        } else {
            SleepTimerTickAction::None
        }
    }

    pub fn state(&self, now_unix_ms: u64) -> SleepTimerState {
        match self.mode {
            SleepTimerMode::Off => SleepTimerState::Off,
            SleepTimerMode::EndOfTrack => SleepTimerState::EndOfTrack,
            SleepTimerMode::Countdown {
                ends_at_unix_ms, ..
            } => {
                let rem_secs = ends_at_unix_ms.saturating_sub(now_unix_ms).div_ceil(1000);
                SleepTimerState::Countdown {
                    remaining_secs: rem_secs,
                }
            }
        }
    }

    pub fn badge_label(&self, now_unix_ms: u64) -> Option<String> {
        match self.state(now_unix_ms) {
            SleepTimerState::Off => None,
            SleepTimerState::EndOfTrack => Some("End of track".to_owned()),
            SleepTimerState::Countdown { remaining_secs } => {
                let mins = remaining_secs / 60;
                let secs = remaining_secs % 60;
                Some(format!("{mins:02}:{secs:02}"))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppTab {
    #[default]
    Home,
    Explore,
    Search,
    Queue,
    NowPlaying,
    Lyrics,
    Library,
    ListenTogether,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DesktopSidePanel {
    #[default]
    Queue,
    Lyrics,
}

#[derive(Clone)]
pub struct UiSlice {
    pub active_tab: AppTab,
    pub nav_history: Vec<AppTab>,
    pub nav_pos: usize,
    pub sidebar_collapsed: bool,
    pub desktop_side_panel_open: bool,
    pub desktop_side_panel: DesktopSidePanel,
    pub status_banner: Option<String>,
    pub active_track_sheet: Option<(Track, Option<String>)>,
    pub add_to_playlist_track: Option<Track>,
    pub text_input_focused: bool,
    pub sleep_timer_sheet_open: bool,
    pub speed_sheet_open: bool,
    pub sleep_timer: SleepTimerController,
}

impl Default for UiSlice {
    fn default() -> Self {
        Self {
            active_tab: AppTab::Home,
            nav_history: vec![AppTab::Home],
            nav_pos: 0,
            sidebar_collapsed: false,
            desktop_side_panel_open: true,
            desktop_side_panel: DesktopSidePanel::Queue,
            status_banner: None,
            active_track_sheet: None,
            add_to_playlist_track: None,
            text_input_focused: false,
            sleep_timer_sheet_open: false,
            speed_sheet_open: false,
            sleep_timer: SleepTimerController::default(),
        }
    }
}

impl UiSlice {
    pub fn set_tab(&mut self, tab: AppTab) {
        if self.active_tab != tab {
            self.nav_history.truncate(self.nav_pos + 1);
            self.nav_history.push(tab);
            self.nav_pos = self.nav_history.len().saturating_sub(1);
            self.active_tab = tab;
        }
    }

    pub fn can_go_back(&self, has_selected_collection: bool) -> bool {
        has_selected_collection || self.nav_pos > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.nav_pos + 1 < self.nav_history.len()
    }

    pub fn go_back(&mut self) {
        if self.nav_pos > 0 {
            self.nav_pos -= 1;
            self.active_tab = self.nav_history[self.nav_pos];
        }
    }

    pub fn go_forward(&mut self) {
        if self.nav_pos + 1 < self.nav_history.len() {
            self.nav_pos += 1;
            self.active_tab = self.nav_history[self.nav_pos];
        }
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
    }

    pub fn toggle_desktop_side_panel(&mut self, panel: DesktopSidePanel) {
        if self.desktop_side_panel_open && self.desktop_side_panel == panel {
            self.desktop_side_panel_open = false;
        } else {
            self.desktop_side_panel = panel;
            self.desktop_side_panel_open = true;
        }
    }

    pub fn set_desktop_side_panel(&mut self, panel: DesktopSidePanel) {
        self.desktop_side_panel = panel;
        self.desktop_side_panel_open = true;
    }

    pub fn set_status_banner(&mut self, msg: impl Into<String>) {
        self.status_banner = Some(msg.into());
    }

    pub fn dismiss_banner(&mut self) {
        self.status_banner = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_history_back_forward_and_truncate() {
        let mut ui = UiSlice::default();
        assert!(!ui.can_go_back(false));
        ui.set_tab(AppTab::Search);
        ui.set_tab(AppTab::Library);
        assert_eq!(ui.active_tab, AppTab::Library);
        assert!(ui.can_go_back(false));
        ui.go_back();
        assert_eq!(ui.active_tab, AppTab::Search);
        assert!(ui.can_go_forward());
        ui.set_tab(AppTab::Settings);
        assert!(!ui.can_go_forward());
        assert_eq!(ui.active_tab, AppTab::Settings);
    }

    #[test]
    fn desktop_side_panel_toggles_cleanly() {
        let mut ui = UiSlice::default();
        assert!(ui.desktop_side_panel_open);
        ui.toggle_desktop_side_panel(DesktopSidePanel::Queue);
        assert!(!ui.desktop_side_panel_open);
        ui.toggle_desktop_side_panel(DesktopSidePanel::Lyrics);
        assert!(ui.desktop_side_panel_open);
        assert_eq!(ui.desktop_side_panel, DesktopSidePanel::Lyrics);
    }
}
