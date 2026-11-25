//! Listen Together rooms, Entity navigation stack (`Artist` / `Album`),
//! and Unified Sign-In sheet state slice (`src/store/rooms_slice.rs`).

use resonance_contract::{AlbumDetail, ArtistDetail, QueuePermission, Track};
use resonance_core::downloads::CollectionOfflineProgress;
use resonance_core::rooms::RoomStateSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityRoute {
    Artist { channel_id: String },
    Album { browse_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListenTogetherSubMode {
    #[default]
    RadioMix,
    Rooms,
}

#[derive(Debug, Clone)]
pub struct RoomsSlice {
    pub sub_mode: ListenTogetherSubMode,
    pub relay_url: String,
    pub room_code_draft: String,
    pub display_name_draft: String,
    pub queue_permission: QueuePermission,
    pub active_room: Option<RoomStateSnapshot>,
    pub joining: bool,
    pub error: Option<String>,
    pub local_track_warning: Option<String>,

    // H2.1 Dedicated Entity navigation stack
    pub entity_history: Vec<EntityRoute>,
    pub entity_pos: usize,
    pub active_artist: Option<ArtistDetail>,
    pub active_album: Option<AlbumDetail>,
    pub album_offline_progress: CollectionOfflineProgress,

    // H3.1 Unified Sign-In Sheet state
    pub auth_sheet_open: bool,
    pub tv_oauth_user_code: Option<String>,
    pub tv_oauth_verification_url: Option<String>,
    pub tv_oauth_expires_in_secs: u64,
    pub tv_oauth_status: Option<String>,
}

impl Default for RoomsSlice {
    fn default() -> Self {
        Self {
            sub_mode: ListenTogetherSubMode::RadioMix,
            relay_url: "https://relay.resonance.dev".to_owned(),
            room_code_draft: String::new(),
            display_name_draft: "Listener".to_owned(),
            queue_permission: QueuePermission::HostOnly,
            active_room: None,
            joining: false,
            error: None,
            local_track_warning: None,
            entity_history: Vec::new(),
            entity_pos: 0,
            active_artist: None,
            active_album: None,
            album_offline_progress: CollectionOfflineProgress::Unpinned,
            auth_sheet_open: false,
            tv_oauth_user_code: None,
            tv_oauth_verification_url: None,
            tv_oauth_expires_in_secs: 0,
            tv_oauth_status: None,
        }
    }
}

impl RoomsSlice {
    pub fn current_entity_route(&self) -> Option<&EntityRoute> {
        self.entity_history.get(self.entity_pos)
    }

    pub fn push_entity_route(&mut self, route: EntityRoute) {
        if self.entity_history.is_empty() {
            self.entity_history.push(route);
            self.entity_pos = 0;
            return;
        }
        if self.entity_history.get(self.entity_pos) == Some(&route) {
            return;
        }
        self.entity_history.truncate(self.entity_pos + 1);
        self.entity_history.push(route);
        self.entity_pos = self.entity_history.len().saturating_sub(1);
    }

    pub fn can_go_back_entity(&self) -> bool {
        !self.entity_history.is_empty()
    }

    pub fn go_back_entity(&mut self) -> Option<EntityRoute> {
        if self.entity_pos > 0 {
            self.entity_pos -= 1;
            self.entity_history.get(self.entity_pos).cloned()
        } else {
            self.entity_history.clear();
            self.entity_pos = 0;
            None
        }
    }

    pub fn clear_entity_routes(&mut self) {
        self.entity_history.clear();
        self.entity_pos = 0;
    }

    pub fn set_room_snapshot(&mut self, snapshot: RoomStateSnapshot) {
        self.local_track_warning = if snapshot.local_track_blocked {
            Some("Local track cannot be synced to remote peers".to_owned())
        } else {
            None
        };
        self.active_room = Some(snapshot);
        self.joining = false;
        self.error = None;
    }

    pub fn leave_room(&mut self) {
        self.active_room = None;
        self.joining = false;
        self.local_track_warning = None;
    }

    pub fn check_local_track_guard(&mut self, track: &Track) -> bool {
        if resonance_core::rooms::is_local_track(&track.id) {
            self.local_track_warning =
                Some("Local track cannot be synced to remote peers".to_owned());
            true
        } else {
            self.local_track_warning = None;
            false
        }
    }
}
