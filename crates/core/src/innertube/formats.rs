//! Decoder-driven audio format preference matrix per target platform and account tier.
//!
//! Format selection is governed by native platform decoder support first, NOT raw bitrate alone:
//! - Android (MediaCodec / Media3 ExoPlayer): full hardware/software Opus + WebM + AAC support.
//! - Desktop (Symphonia / rodio AAC-first `.m4a` path, optional Opus/WebM fallback).
//! - iOS (`AVPlayer`): AAC/MP4 only — WebM Opus (`774`, `251`) is explicitly excluded.

use crate::innertube::matrix::AccountTier;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetPlatform {
    Android,
    Desktop,
    Ios,
}

impl TargetPlatform {
    /// Detects the active compilation target platform.
    pub const fn current() -> Self {
        #[cfg(target_os = "android")]
        {
            Self::Android
        }
        #[cfg(target_os = "ios")]
        {
            Self::Ios
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            Self::Desktop
        }
    }

    /// Returns the ordered list of preferred YouTube audio `itag`s gated by native decoder support
    /// and account tier (Free vs Premium 256 kbps).
    ///
    /// - `774`: Opus ~256 kbps (WebM, Premium)
    /// - `251`: Opus ~128–160 kbps (WebM, Free)
    /// - `141`: AAC-LC ~256 kbps (MP4 / `.m4a`, Premium)
    /// - `140`: AAC-LC ~128 kbps (MP4 / `.m4a`, Free)
    /// - `139`: HE-AAC ~48 kbps (MP4 / `.m4a`, low-bandwidth fallback)
    pub const fn preferred_audio_itags(self, tier: AccountTier) -> &'static [u32] {
        match (self, tier) {
            // Android MediaCodec handles Opus natively with best perceptual quality per byte.
            (Self::Android, AccountTier::PremiumSignedIn) => &[774, 141, 251, 140, 139],
            (Self::Android, AccountTier::FreeSignedIn | AccountTier::Anonymous) => &[251, 140, 139],

            // Desktop (Symphonia / rodio): AAC-first `.m4a` container compatibility, Opus fallback.
            (Self::Desktop, AccountTier::PremiumSignedIn) => &[141, 774, 140, 251, 139],
            (Self::Desktop, AccountTier::FreeSignedIn | AccountTier::Anonymous) => &[140, 251, 139],

            // iOS AVPlayer does NOT natively play WebM Opus — strictly AAC/MP4 itags only!
            (Self::Ios, AccountTier::PremiumSignedIn) => &[141, 140, 139],
            (Self::Ios, AccountTier::FreeSignedIn | AccountTier::Anonymous) => &[140, 139],
        }
    }

    /// Returns `true` if the target's native audio stack supports WebM/Opus streams.
    pub const fn supports_webm_opus(self) -> bool {
        !matches!(self, Self::Ios)
    }
}
