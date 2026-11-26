//! Listen Together rooms state slice (`src/store/rooms_slice.rs`).
//!
//! Scoped strictly to synchronized listening rooms (host/join, permissions, active
//! room snapshot, and local-track sync warnings).

use resonance_contract::{QueuePermission, Track};
use resonance_core::rooms::RoomStateSnapshot;

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
        }
    }
}

impl RoomsSlice {
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
