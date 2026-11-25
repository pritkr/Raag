//! Dual-buffer slot manager for Proxy v2 (`current_track` + `next_track`).
//!
//! Keeps hot metadata & pre-warm state for the active track and the upcoming
//! track (pre-warmed at >= 75% playback progress) while maintaining full
//! compatibility with arbitrary stream registrations.

use std::collections::HashMap;

use resonance_contract::StreamInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferSlot {
    pub video_id: String,
    pub itag: u32,
    pub warmed_chunks: u64,
    pub content_length: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DualBufferStatus {
    pub current: Option<BufferSlot>,
    pub next: Option<BufferSlot>,
}

/// Manages `current_track` and `next_track` dual-buffer slots alongside a
/// backing stream registry.
#[derive(Debug, Default)]
pub struct DualBufferSlots {
    current: Option<StreamInfo>,
    current_warmed_chunks: u64,
    next: Option<StreamInfo>,
    next_warmed_chunks: u64,
    extra: HashMap<String, StreamInfo>,
}

impl DualBufferSlots {
    pub fn set_current(&mut self, stream: StreamInfo) {
        // If the promoted stream was already warmed in `next`, carry over its warmed chunk count!
        let warmed = if self
            .next
            .as_ref()
            .is_some_and(|n| n.video_id == stream.video_id && n.itag == stream.itag)
        {
            self.next = None;
            std::mem::take(&mut self.next_warmed_chunks)
        } else {
            0
        };
        self.extra.insert(stream.video_id.clone(), stream.clone());
        self.current = Some(stream);
        self.current_warmed_chunks = warmed;
    }

    pub fn set_next(&mut self, stream: StreamInfo) {
        let keep_warmed = self
            .next
            .as_ref()
            .is_some_and(|n| n.video_id == stream.video_id && n.itag == stream.itag);
        if !keep_warmed {
            self.next_warmed_chunks = 0;
        }
        self.extra.insert(stream.video_id.clone(), stream.clone());
        self.next = Some(stream);
    }

    /// Registers a stream without forcing slot reassignment if it is already in a slot.
    pub fn register(&mut self, stream: StreamInfo) {
        if self
            .current
            .as_ref()
            .is_none_or(|c| c.video_id == stream.video_id)
        {
            self.set_current(stream);
        } else if self
            .next
            .as_ref()
            .is_some_and(|n| n.video_id == stream.video_id)
        {
            self.set_next(stream);
        } else {
            self.extra.insert(stream.video_id.clone(), stream);
        }
    }

    pub fn record_warmed_chunk(&mut self, video_id: &str, chunk_count: u64) {
        if self
            .current
            .as_ref()
            .is_some_and(|c| c.video_id == video_id)
        {
            self.current_warmed_chunks = self.current_warmed_chunks.max(chunk_count);
        }
        if self.next.as_ref().is_some_and(|n| n.video_id == video_id) {
            self.next_warmed_chunks = self.next_warmed_chunks.max(chunk_count);
        }
    }

    pub fn get(&self, video_id: &str) -> Option<StreamInfo> {
        if let Some(c) = &self.current
            && c.video_id == video_id
        {
            return Some(c.clone());
        }
        if let Some(n) = &self.next
            && n.video_id == video_id
        {
            return Some(n.clone());
        }
        self.extra.get(video_id).cloned()
    }

    pub fn remove(&mut self, video_id: &str) {
        if self
            .current
            .as_ref()
            .is_some_and(|c| c.video_id == video_id)
        {
            self.current = None;
            self.current_warmed_chunks = 0;
        }
        if self.next.as_ref().is_some_and(|n| n.video_id == video_id) {
            self.next = None;
            self.next_warmed_chunks = 0;
        }
        self.extra.remove(video_id);
    }

    pub fn status(&self) -> DualBufferStatus {
        DualBufferStatus {
            current: self.current.as_ref().map(|s| BufferSlot {
                video_id: s.video_id.clone(),
                itag: s.itag,
                warmed_chunks: self.current_warmed_chunks,
                content_length: s.content_length,
            }),
            next: self.next.as_ref().map(|s| BufferSlot {
                video_id: s.video_id.clone(),
                itag: s.itag,
                warmed_chunks: self.next_warmed_chunks,
                content_length: s.content_length,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use resonance_contract::AudioCodec;

    fn sample_stream(id: &str) -> StreamInfo {
        StreamInfo {
            video_id: id.to_owned(),
            url: format!("https://cdn.example/{id}"),
            itag: 251,
            mime_type: "audio/webm".into(),
            codec: AudioCodec::Opus,
            bitrate_kbps: 160,
            content_length: 1024 * 1024,
            duration_ms: 180_000,
            loudness_db: Some(-1.5),
            resolved_by_client: "ANDROID_VR".into(),
            expires_at_unix_secs: 9_999_999,
        }
    }

    #[test]
    fn promotes_warmed_next_slot_into_current_slot() {
        let mut slots = DualBufferSlots::default();
        slots.set_current(sample_stream("track_a"));
        slots.set_next(sample_stream("track_b"));
        slots.record_warmed_chunk("track_b", 2);

        let before = slots.status();
        assert_eq!(before.current.as_ref().unwrap().video_id, "track_a");
        assert_eq!(before.next.as_ref().unwrap().video_id, "track_b");
        assert_eq!(before.next.as_ref().unwrap().warmed_chunks, 2);

        slots.set_current(sample_stream("track_b"));
        let after = slots.status();
        assert_eq!(after.current.as_ref().unwrap().video_id, "track_b");
        assert_eq!(after.current.as_ref().unwrap().warmed_chunks, 2);
        assert!(after.next.is_none());
    }
}
