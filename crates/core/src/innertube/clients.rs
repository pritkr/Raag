//! Centralized InnerTube client configuration table (2026 Ground Truth).
//!
//! ALL `clientName`, `clientVersion`, `userAgent`, `clientId`, and `AuthMode`
//! rules live exclusively in this module — never scattered across parsers.

use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientKind {
    /// Primary Browse / Search / Home / Library / Next + last-resort playback fallback.
    WebRemix,
    /// Primary Playback: plain byte-rangeable stream URLs (no JS cipher, no PO token).
    VisionOs,
    /// Playback Fallback ONLY when TV OAuth is active (NEVER receives browser cookies).
    TvSamsung,
    /// Anonymous Playback Fallback (NEVER receives browser cookies).
    AndroidVr,
    /// Avoid for logged-in sessions (silently drops cookies).
    Ios,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    /// May attach `SAPISIDHASH` + browser cookies, or run anonymously.
    CookiesOrAnonymous,
    /// ONLY accepts TV OAuth Bearer tokens; browser cookies are strictly forbidden.
    TvOAuthOnly,
    /// Strictly anonymous — NEVER receives browser cookies or `SAPISIDHASH`.
    AnonymousOnly,
}

#[derive(Debug, Clone, Copy)]
pub struct ClientConfig {
    pub kind: ClientKind,
    pub client_name: &'static str,
    pub client_id: &'static str,
    pub client_version: &'static str,
    pub user_agent: &'static str,
    pub origin: &'static str,
    pub api_base: &'static str,
    pub auth_mode: AuthMode,
    pub needs_js_solver: bool,
    pub device_make: Option<&'static str>,
    pub device_model: Option<&'static str>,
    pub os_name: Option<&'static str>,
    pub os_version: Option<&'static str>,
    pub android_sdk_version: Option<u32>,
}

pub const WEB_REMIX: ClientConfig = ClientConfig {
    kind: ClientKind::WebRemix,
    client_name: "WEB_REMIX",
    client_id: "67",
    client_version: "1.20261006.10.00",
    user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
    origin: "https://music.youtube.com",
    api_base: "https://music.youtube.com/youtubei/v1",
    auth_mode: AuthMode::CookiesOrAnonymous,
    needs_js_solver: true,
    device_make: None,
    device_model: None,
    os_name: None,
    os_version: None,
    android_sdk_version: None,
};

pub const VISIONOS: ClientConfig = ClientConfig {
    kind: ClientKind::VisionOs,
    client_name: "VISIONOS",
    client_id: "101",
    client_version: "1.02",
    user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15",
    origin: "https://www.youtube.com",
    api_base: "https://www.youtube.com/youtubei/v1",
    auth_mode: AuthMode::CookiesOrAnonymous,
    needs_js_solver: false,
    device_make: Some("Apple"),
    device_model: Some("RealityDevice17,1"),
    os_name: Some("visionOS"),
    os_version: Some("26.5.23O471"),
    android_sdk_version: None,
};

pub const ANDROID_VR: ClientConfig = ClientConfig {
    kind: ClientKind::AndroidVr,
    client_name: "ANDROID_VR",
    client_id: "28",
    client_version: "1.73.21",
    user_agent: "com.google.android.apps.youtube.vr.pico/1.73.21 (Linux; U; Android 10; A8110 Build/QKQ1) gzip",
    origin: "https://www.youtube.com",
    api_base: "https://youtubei.googleapis.com/youtubei/v1",
    auth_mode: AuthMode::AnonymousOnly,
    needs_js_solver: false,
    device_make: Some("Pico"),
    device_model: Some("A8110"),
    os_name: Some("Android"),
    os_version: Some("10"),
    android_sdk_version: Some(29),
};

pub const TV_SAMSUNG: ClientConfig = ClientConfig {
    kind: ClientKind::TvSamsung,
    client_name: "TVHTML5",
    client_id: "7",
    client_version: "7.20260707.12.00",
    user_agent: "Mozilla/5.0 (SMART-TV; LINUX; Tizen 7.0) AppleWebKit/537.36 (KHTML, like Gecko) Version/7.0 TV Safari/537.36",
    origin: "https://www.youtube.com",
    api_base: "https://www.youtube.com/youtubei/v1",
    auth_mode: AuthMode::TvOAuthOnly,
    needs_js_solver: true,
    device_make: Some("Samsung"),
    device_model: Some("SMART-TV"),
    os_name: Some("Tizen"),
    os_version: Some("7.0"),
    android_sdk_version: None,
};

pub const IOS: ClientConfig = ClientConfig {
    kind: ClientKind::Ios,
    client_name: "IOS",
    client_id: "5",
    client_version: "20.10.4",
    user_agent: "com.google.ios.youtube/20.10.4 (iPhone16,2; U; CPU iOS 18_3 like Mac OS X;)",
    origin: "https://www.youtube.com",
    api_base: "https://youtubei.googleapis.com/youtubei/v1",
    auth_mode: AuthMode::AnonymousOnly,
    needs_js_solver: false,
    device_make: Some("Apple"),
    device_model: Some("iPhone16,2"),
    os_name: Some("iPhone"),
    os_version: Some("18.3"),
    android_sdk_version: None,
};

impl ClientConfig {
    pub fn for_kind(kind: ClientKind) -> &'static ClientConfig {
        match kind {
            ClientKind::WebRemix => &WEB_REMIX,
            ClientKind::VisionOs => &VISIONOS,
            ClientKind::TvSamsung => &TV_SAMSUNG,
            ClientKind::AndroidVr => &ANDROID_VR,
            ClientKind::Ios => &IOS,
        }
    }

    /// Strict compile/runtime guard: returns `true` ONLY if browser cookies (`SAPISIDHASH`)
    /// are legally permitted on this client.
    pub fn allows_browser_cookies(&self) -> bool {
        matches!(self.auth_mode, AuthMode::CookiesOrAnonymous)
    }

    pub fn allows_tv_oauth(&self) -> bool {
        matches!(self.auth_mode, AuthMode::TvOAuthOnly)
    }

    /// Builds the InnerTube `context.client` JSON payload with optional `visitorData`.
    pub fn build_context(&self, visitor_data: Option<&str>, hl: &str, gl: &str) -> Value {
        let mut client = json!({
            "clientName": self.client_name,
            "clientVersion": self.client_version,
            "hl": hl,
            "gl": gl,
        });
        if let Some(obj) = client.as_object_mut() {
            if let Some(v) = visitor_data.filter(|s| !s.is_empty()) {
                obj.insert("visitorData".into(), json!(v));
            }
            if let Some(make) = self.device_make {
                obj.insert("deviceMake".into(), json!(make));
            }
            if let Some(model) = self.device_model {
                obj.insert("deviceModel".into(), json!(model));
            }
            if let Some(os) = self.os_name {
                obj.insert("osName".into(), json!(os));
            }
            if let Some(ver) = self.os_version {
                obj.insert("osVersion".into(), json!(ver));
            }
            if let Some(sdk) = self.android_sdk_version {
                obj.insert("androidSdkVersion".into(), json!(sdk));
            }
            if matches!(self.kind, ClientKind::VisionOs | ClientKind::WebRemix) {
                obj.insert("userAgent".into(), json!(self.user_agent));
            }
        }
        json!({ "client": client })
    }
}
