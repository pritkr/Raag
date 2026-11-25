//! Navigation, sidebar/panel chrome, and global status toast slice (`UiSlice`).

use resonance_contract::Track;

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
