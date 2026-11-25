//! Cross-platform LAN Casting (Chromecast CastV2, DLNA/UPnP, AirPlay) with
//! session-scoped, IP-pinned `LanCastRelay`.

pub mod airplay;
pub mod chromecast;
pub mod discovery;
pub mod dlna;
pub mod manager;

pub use airplay::AirPlayTransport;
pub use chromecast::{CastEvent, CastMessage, ChromecastTransport};
pub use discovery::{CastDiscoveryService, validate_ssdp_location_url};
pub use dlna::{DlnaTransport, build_didl_lite_metadata, xml_escape};
pub use manager::{CastManager, CastPlaybackDelegate};
