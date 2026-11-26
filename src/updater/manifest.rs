//! Signed update manifest schema (`latest.json` / `beta.json`).

use std::collections::BTreeMap;

use resonance_contract::UpdateChannel;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformArtifact {
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub schema_version: u32,
    pub version: String,
    pub channel: String,
    pub published_at_unix_secs: u64,
    pub release_notes_md: String,
    pub platforms: BTreeMap<String, PlatformArtifact>,
}

impl UpdateManifest {
    pub fn artifact_for_platform(&self, platform_key: &str) -> Option<&PlatformArtifact> {
        self.platforms.get(platform_key)
    }

    pub fn artifact_for_current_platform(&self) -> Option<&PlatformArtifact> {
        self.artifact_for_platform(current_platform_key())
    }
}

pub fn current_platform_key() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "darwin-aarch64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "darwin-x86_64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "windows-x86_64"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux-x86_64"
    }
    #[cfg(target_os = "android")]
    {
        "android-arm64"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        target_os = "android"
    )))]
    {
        "unknown"
    }
}

pub fn channel_manifest_filename(channel: UpdateChannel) -> &'static str {
    match channel {
        UpdateChannel::Stable => "latest.json",
        UpdateChannel::Beta => "beta.json",
    }
}
