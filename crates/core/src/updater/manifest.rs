//! Signed update manifest parsing, fail-closed Ed25519 verification, SemVer ordering,
//! and strict HTTPS URL enforcement for the Raag Native Auto-Updater (`P7`).

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use resonance_contract::{UpdateChannel, UpdateInfo};
use serde::{Deserialize, Serialize};
use url::Url;

/// Maximum signed manifest size accepted over the network (256 KB).
pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;

/// Default HTTPS release feed URLs per channel (aligned with `nsh.json`, overridable via env).
pub const DEFAULT_STABLE_MANIFEST_URL: &str =
    "https://github.com/pritkr/Raag/releases/latest/download/latest.json";
pub const DEFAULT_BETA_MANIFEST_URL: &str =
    "https://github.com/pritkr/Raag/releases/download/beta-latest/latest.json";

/// Compile-time Ed25519 verifying key (hex, 32 bytes), aligned with `nsh.json`'s `updater.publicKey`.
/// Override at build time via `RAAG_UPDATER_ED25519_PUBKEY_HEX`.
pub const EMBEDDED_UPDATER_PUBKEY_HEX: Option<&str> =
    match option_env!("RAAG_UPDATER_ED25519_PUBKEY_HEX") {
        Some(v) => Some(v),
        None => match option_env!("RESONANCE_UPDATER_ED25519_PUBKEY_HEX") {
            Some(v) => Some(v),
            None => Some("5c20058cbec80206445ff3e43e341a97e6ebfffc1fb4c27197e1acb63f7ed335"),
        },
    };

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedUpdateManifest {
    pub schema_version: u32,
    pub version: String,
    pub channel: UpdateChannel,
    pub released_at_unix_secs: u64,
    pub notes_markdown: String,
    pub artifacts: Vec<ReleaseArtifactEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseArtifactEntry {
    /// Target identifier matching `current_platform_target()` (e.g. `linux-x86_64`, `macos-aarch64`,
    /// `windows-x86_64`, `android-arm64`).
    pub target: String,
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease_rank: u8,
    pub prerelease_num: u64,
}

impl SemVer {
    /// Parses `v1.2.3`, `1.2.3-beta.4`, `1.2.3-rc.1`, or `1.2.3-alpha.2`.
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim().trim_start_matches(['v', 'V']);
        let (core, pre) = match trimmed.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (trimmed, None),
        };
        let mut parts = core.split('.');
        let major = parts.next()?.parse::<u64>().ok()?;
        let minor = parts.next()?.parse::<u64>().ok()?;
        let patch = parts.next()?.parse::<u64>().ok()?;
        if parts.next().is_some() {
            return None;
        }
        let (prerelease_rank, prerelease_num) = match pre {
            None => (3, 0),
            Some(p) => {
                let lower = p.to_ascii_lowercase();
                let num = lower
                    .split(['.', '-', '_'])
                    .next_back()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                if lower.starts_with("rc") {
                    (2, num)
                } else if lower.starts_with("beta") {
                    (1, num)
                } else {
                    (0, num)
                }
            }
        };
        Some(Self {
            major,
            minor,
            patch,
            prerelease_rank,
            prerelease_num,
        })
    }
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            self.major,
            self.minor,
            self.patch,
            self.prerelease_rank,
            self.prerelease_num,
        )
            .cmp(&(
                other.major,
                other.minor,
                other.patch,
                other.prerelease_rank,
                other.prerelease_num,
            ))
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Returns `true` if `candidate` is strictly newer than `current` under SemVer rules.
pub fn is_newer_version(current: &str, candidate: &str) -> bool {
    match (SemVer::parse(current), SemVer::parse(candidate)) {
        (Some(cur), Some(cand)) => cand > cur,
        _ => false,
    }
}

/// Canonical platform target slug for matching release manifest artifacts.
pub fn current_platform_target() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux-x86_64"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "linux-aarch64"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "macos-aarch64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "macos-x86_64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "windows-x86_64"
    }
    #[cfg(all(target_os = "android", target_arch = "aarch64"))]
    {
        "android-arm64"
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "android", target_arch = "aarch64"),
    )))]
    {
        "unknown"
    }
}

/// Validates that an updater URL uses HTTPS (or loopback HTTP strictly when `allow_loopback_http`
/// is enabled for deterministic local integration tests).
pub fn validate_updater_url(raw: &str, allow_loopback_http: bool) -> Result<Url, String> {
    let parsed = Url::parse(raw).map_err(|e| format!("invalid updater URL: {e}"))?;
    match parsed.scheme() {
        "https" => Ok(parsed),
        "http" if allow_loopback_http => {
            let host = parsed.host_str().unwrap_or("");
            if host == "127.0.0.1" || host == "localhost" {
                Ok(parsed)
            } else {
                Err("updater requires HTTPS for non-loopback hosts".to_owned())
            }
        }
        _ => Err("updater rejects non-HTTPS URLs".to_owned()),
    }
}

/// Decodes a 32-byte Ed25519 verifying key from hex (or returns an error if invalid).
pub fn parse_verifying_key_hex(pubkey_hex: &str) -> Result<VerifyingKey, String> {
    let raw = hex::decode(pubkey_hex.trim())
        .map_err(|e| format!("invalid Ed25519 public key hex: {e}"))?;
    let bytes: [u8; 32] = raw
        .try_into()
        .map_err(|_| "Ed25519 public key must be exactly 32 bytes".to_owned())?;
    VerifyingKey::from_bytes(&bytes).map_err(|e| format!("invalid Ed25519 public key: {e}"))
}

/// Fail-closed Ed25519 verification over raw manifest bytes BEFORE JSON deserialization.
pub fn verify_and_parse_manifest(
    manifest_bytes: &[u8],
    sig_bytes: &[u8],
    pubkey: &VerifyingKey,
    allow_loopback_http: bool,
) -> Result<SignedUpdateManifest, String> {
    if manifest_bytes.is_empty() || manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err("update manifest size out of bounds".to_owned());
    }
    let sig_arr: [u8; 64] = if sig_bytes.len() == 64 {
        sig_bytes.try_into().unwrap()
    } else {
        // Also accept 128-char hex-encoded signature files (matching `sign_config` `.sig` format).
        let trimmed = std::str::from_utf8(sig_bytes)
            .map_err(|_| "signature must be 64 raw bytes or 128 hex chars".to_owned())?
            .trim();
        let decoded =
            hex::decode(trimmed).map_err(|_| "invalid hex-encoded Ed25519 signature".to_owned())?;
        decoded
            .try_into()
            .map_err(|_| "decoded Ed25519 signature must be 64 bytes".to_owned())?
    };
    let signature = Signature::from_bytes(&sig_arr);
    pubkey
        .verify(manifest_bytes, &signature)
        .map_err(|_| "Ed25519 signature verification failed for update manifest".to_owned())?;

    let manifest: SignedUpdateManifest = serde_json::from_slice(manifest_bytes)
        .map_err(|e| format!("verified manifest JSON invalid: {e}"))?;
    if manifest.schema_version != 1 {
        return Err(format!(
            "unsupported update manifest schema version: {}",
            manifest.schema_version
        ));
    }
    if SemVer::parse(&manifest.version).is_none() {
        return Err(format!("invalid SemVer in manifest: {}", manifest.version));
    }
    for art in &manifest.artifacts {
        validate_updater_url(&art.url, allow_loopback_http)?;
        if art.sha256.len() != 64 || !art.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("invalid artifact sha256 for target {}", art.target));
        }
    }
    Ok(manifest)
}

/// Converts a verified manifest into an [`UpdateInfo`] for `target` if it is strictly newer
/// than `current_version` and allowed for `channel`.
pub fn select_update_for_target(
    manifest: &SignedUpdateManifest,
    current_version: &str,
    channel: UpdateChannel,
    target: &str,
    can_install_in_place: bool,
) -> Option<UpdateInfo> {
    if channel == UpdateChannel::Stable && manifest.channel != UpdateChannel::Stable {
        return None;
    }
    if !is_newer_version(current_version, &manifest.version) {
        return None;
    }
    let artifact = manifest.artifacts.iter().find(|a| a.target == target)?;
    Some(UpdateInfo {
        version: manifest.version.clone(),
        release_notes: manifest.notes_markdown.clone(),
        download_url: artifact.url.clone(),
        can_install_in_place,
        published_at: Some(manifest.released_at_unix_secs.to_string()),
        sha256_hex: Some(artifact.sha256.to_ascii_lowercase()),
    })
}
