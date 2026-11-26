//! Next-track stream pre-resolution (>= 50%) and Proxy v2 dual-buffer warm-up (>= 75%).

use std::sync::Arc;

use resonance_contract::{MusicProvider, OfflineStore};

use crate::proxy::LoopbackProxy;

/// Number of initial 256 KB chunks (4 × 256 KB = 1 MB) pre-warmed into the Proxy v2 `next_track` slot at >= 75%.
pub const NEXT_TRACK_PREWARM_CHUNKS: u64 = 4;

/// Pure prefetch decision returned by [`PrefetchThresholds::evaluate`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PrefetchDecision {
    #[default]
    None,
    /// Reached >= 50%: resolve stream URL & register in Proxy v2 `next_track` slot.
    ResolveOnly { video_id: String },
    /// Reached >= 75%: resolve if needed and pre-warm initial 256 KB chunks into `ChunkCache`.
    WarmProxySlot { video_id: String },
}

pub struct PrefetchThresholds;

impl PrefetchThresholds {
    /// Evaluates pure 50% (metadata resolution) and 75% (Proxy v2 chunk pre-warm) thresholds.
    pub fn evaluate(
        position_ms: u64,
        duration_ms: u64,
        upcoming_video_id: Option<&str>,
        already_preresolved: Option<&str>,
        already_prewarmed: Option<&str>,
    ) -> PrefetchDecision {
        if duration_ms == 0 {
            return PrefetchDecision::None;
        }
        let Some(next_id) = upcoming_video_id else {
            return PrefetchDecision::None;
        };

        // >= 75% progress: warm Proxy v2 dual-buffer next_track slot
        if position_ms.saturating_mul(4) >= duration_ms.saturating_mul(3) {
            if already_prewarmed != Some(next_id) {
                return PrefetchDecision::WarmProxySlot {
                    video_id: next_id.to_owned(),
                };
            }
            return PrefetchDecision::None;
        }

        // >= 50% progress: resolve next track stream metadata
        if position_ms.saturating_mul(2) >= duration_ms && already_preresolved != Some(next_id) {
            return PrefetchDecision::ResolveOnly {
                video_id: next_id.to_owned(),
            };
        }

        PrefetchDecision::None
    }
}

/// Async coordinator executing [`PrefetchDecision`] off the critical playback path.
pub struct PrefetchCoordinator;

impl PrefetchCoordinator {
    pub fn spawn_for_decision(
        decision: PrefetchDecision,
        provider: Arc<dyn MusicProvider>,
        proxy: Arc<LoopbackProxy>,
        offline: Option<Arc<dyn OfflineStore>>,
    ) {
        match decision {
            PrefetchDecision::None => {}
            PrefetchDecision::ResolveOnly { video_id } => {
                if Self::is_completed_offline(&offline, &video_id) {
                    return;
                }
                tokio::spawn(async move {
                    if let Ok(stream_info) = provider.resolve_stream(&video_id).await {
                        proxy.register_next_stream(stream_info);
                    }
                });
            }
            PrefetchDecision::WarmProxySlot { video_id } => {
                if Self::is_completed_offline(&offline, &video_id) {
                    return;
                }
                tokio::spawn(async move {
                    if let Ok(stream_info) = provider.resolve_stream(&video_id).await {
                        let _ = proxy
                            .warm_next_track(stream_info, NEXT_TRACK_PREWARM_CHUNKS)
                            .await;
                    }
                });
            }
        }
    }

    fn is_completed_offline(offline: &Option<Arc<dyn OfflineStore>>, video_id: &str) -> bool {
        offline
            .as_ref()
            .and_then(|o| o.get_completed_download(video_id).ok().flatten())
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_50_and_75_percent_boundaries_exactly() {
        assert_eq!(
            PrefetchThresholds::evaluate(49_999, 100_000, Some("next_v"), None, None),
            PrefetchDecision::None
        );
        assert_eq!(
            PrefetchThresholds::evaluate(50_000, 100_000, Some("next_v"), None, None),
            PrefetchDecision::ResolveOnly {
                video_id: "next_v".into()
            }
        );
        assert_eq!(
            PrefetchThresholds::evaluate(74_999, 100_000, Some("next_v"), Some("next_v"), None),
            PrefetchDecision::None
        );
        assert_eq!(
            PrefetchThresholds::evaluate(75_000, 100_000, Some("next_v"), Some("next_v"), None),
            PrefetchDecision::WarmProxySlot {
                video_id: "next_v".into()
            }
        );
        // Jumping directly to >= 75% skips ResolveOnly and triggers WarmProxySlot directly
        assert_eq!(
            PrefetchThresholds::evaluate(80_000, 100_000, Some("next_v"), None, None),
            PrefetchDecision::WarmProxySlot {
                video_id: "next_v".into()
            }
        );
    }
}
