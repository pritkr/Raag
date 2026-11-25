//! Pure queue manipulation logic (shuffle, repeat, up-next, reorder, radio dedup).
//!
//! Zero network or async I/O — 100% deterministic and unit-testable.

use std::collections::HashSet;

use rand::seq::SliceRandom;
use resonance_contract::{CoreError, CoreResult, PlaybackError, QueueState, RepeatMode, Track};

/// Stateless pure operations over [`QueueState`].
pub struct QueueManager;

impl QueueManager {
    /// Replaces the queue with `tracks` and focuses `start_index`.
    pub fn replace(
        queue: &mut QueueState,
        tracks: Vec<Track>,
        start_index: usize,
    ) -> CoreResult<()> {
        if tracks.is_empty() {
            return Err(CoreError::Playback(PlaybackError::EmptyQueue));
        }
        let idx = start_index.min(tracks.len() - 1);
        queue.items = tracks;
        queue.current_index = Some(idx);
        queue.resume_position_ms = 0;
        Ok(())
    }

    /// Inserts `track` immediately after the current track (or at index 0 if empty).
    pub fn play_next(queue: &mut QueueState, track: Track) {
        let insert_at = queue
            .current_index
            .map(|i| (i + 1).min(queue.items.len()))
            .unwrap_or(0);
        queue.items.insert(insert_at, track);
        if queue.current_index.is_none() {
            queue.current_index = Some(0);
        }
    }

    /// Appends `track` to the tail of the queue.
    pub fn play_last(queue: &mut QueueState, track: Track) {
        queue.items.push(track);
        if queue.current_index.is_none() {
            queue.current_index = Some(0);
        }
    }

    /// Moves an item from `from` to `to` while preserving `current_index` pointer semantics.
    pub fn reorder(queue: &mut QueueState, from: usize, to: usize) {
        let len = queue.items.len();
        if from >= len || to >= len || from == to {
            return;
        }
        let item = queue.items.remove(from);
        queue.items.insert(to, item);

        if let Some(cur) = queue.current_index {
            queue.current_index = Some(if cur == from {
                to
            } else if from < cur && cur <= to {
                cur - 1
            } else if to <= cur && cur < from {
                cur + 1
            } else {
                cur
            });
        }
    }

    /// Removes `index` from the queue and returns whether the current track changed or queue emptied.
    pub fn remove(queue: &mut QueueState, index: usize) -> Option<Option<Track>> {
        if index >= queue.items.len() {
            return None;
        }
        queue.items.remove(index);
        if queue.items.is_empty() {
            queue.current_index = None;
            return Some(None);
        }
        if let Some(cur) = queue.current_index {
            if index < cur {
                queue.current_index = Some(cur - 1);
            } else if index == cur {
                let next_idx = cur.min(queue.items.len() - 1);
                queue.current_index = Some(next_idx);
                return Some(queue.items.get(next_idx).cloned());
            }
        }
        None
    }

    /// Clears queue items while preserving shuffle, repeat, and autoplay radio preferences.
    pub fn clear_preserving_modes(queue: &mut QueueState) {
        *queue = QueueState {
            shuffle: queue.shuffle,
            repeat: queue.repeat,
            autoplay_radio: queue.autoplay_radio,
            ..QueueState::default()
        };
    }

    /// Toggles shuffle; when turned ON, shuffles only the upcoming tail after `current_index`.
    pub fn toggle_shuffle(queue: &mut QueueState) {
        queue.shuffle = !queue.shuffle;
        if queue.shuffle && queue.items.len() > 2 {
            let cur_idx = queue.current_index.unwrap_or(0);
            let tail_start = (cur_idx + 1).min(queue.items.len());
            let mut rng = rand::rng();
            queue.items[tail_start..].shuffle(&mut rng);
        }
    }

    pub fn cycle_repeat(queue: &mut QueueState) -> RepeatMode {
        queue.repeat = queue.repeat.cycle();
        queue.repeat
    }

    /// Computes the next queue index according to `RepeatMode`.
    pub fn resolve_next_index(queue: &QueueState) -> CoreResult<Option<usize>> {
        let len = queue.items.len();
        if len == 0 {
            return Err(CoreError::Playback(PlaybackError::EmptyQueue));
        }
        let cur = queue.current_index.unwrap_or(0);
        Ok(match queue.repeat {
            RepeatMode::One => Some(cur),
            RepeatMode::All => Some((cur + 1) % len),
            RepeatMode::None => (cur + 1 < len).then_some(cur + 1),
        })
    }

    /// Computes the previous queue index (assuming position <= 3s restart threshold).
    pub fn resolve_previous_index(queue: &QueueState) -> CoreResult<usize> {
        let len = queue.items.len();
        if len == 0 {
            return Err(CoreError::Playback(PlaybackError::EmptyQueue));
        }
        let cur = queue.current_index.unwrap_or(0);
        Ok(if cur > 0 {
            cur - 1
        } else if queue.repeat == RepeatMode::All {
            len - 1
        } else {
            0
        })
    }

    /// Peeks the upcoming track ID that should be pre-resolved/pre-warmed for gapless handoff.
    pub fn peek_upcoming_track_id(queue: &QueueState) -> Option<String> {
        let len = queue.items.len();
        let next_idx = queue.current_index.and_then(|cur| match queue.repeat {
            RepeatMode::One => None,
            RepeatMode::All if len > 1 => Some((cur + 1) % len),
            RepeatMode::None if cur + 1 < len => Some(cur + 1),
            _ => None,
        });
        next_idx
            .and_then(|idx| queue.items.get(idx))
            .map(|t| t.id.clone())
    }

    /// Merges autoplay radio tracks into the tail of the queue, deduplicating existing IDs
    /// and interleaving artists for YouTube Music-style variety.
    /// Returns the index of the first newly appended track if any were added.
    pub fn merge_radio_tracks(queue: &mut QueueState, radio: Vec<Track>) -> Option<usize> {
        let seed = queue.current_track().cloned();
        let diversified = Self::diversify_radio_tracks(seed.as_ref(), &queue.items, radio);
        let start = queue.items.len();
        queue.items.extend(diversified);
        (queue.items.len() > start).then_some(start)
    }

    /// Deduplicates and interleaves candidate radio tracks so no single artist dominates
    /// consecutive Up Next slots, while preserving relevance ordering.
    pub fn diversify_radio_tracks(
        seed: Option<&Track>,
        existing_queue: &[Track],
        candidates: Vec<Track>,
    ) -> Vec<Track> {
        fn norm_artist(t: &Track) -> String {
            let primary = t
                .artists
                .first()
                .map(|a| a.name.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or("");
            if !primary.is_empty() {
                return primary.to_lowercase();
            }
            let disp = t.artist_display();
            let trimmed = disp.trim();
            if trimmed.eq_ignore_ascii_case("unknown artist") {
                String::new()
            } else {
                trimmed.to_lowercase()
            }
        }

        fn norm_title_key(t: &Track) -> (String, String) {
            let title = t.title.trim().to_lowercase();
            (title, norm_artist(t))
        }

        let mut seen_ids: HashSet<String> = existing_queue.iter().map(|t| t.id.clone()).collect();
        let mut seen_titles: HashSet<(String, String)> = existing_queue
            .iter()
            .filter(|t| !t.title.trim().is_empty())
            .map(norm_title_key)
            .collect();
        if let Some(s) = seed {
            seen_ids.insert(s.id.clone());
            if !s.title.trim().is_empty() {
                seen_titles.insert(norm_title_key(s));
            }
        }

        let mut pool: Vec<Track> = Vec::with_capacity(candidates.len());
        for t in candidates {
            if t.id.is_empty() || !seen_ids.insert(t.id.clone()) {
                continue;
            }
            let tkey = norm_title_key(&t);
            if !tkey.0.is_empty() && !tkey.1.is_empty() && !seen_titles.insert(tkey) {
                continue;
            }
            pool.push(t);
        }

        if pool.len() <= 2 {
            return pool;
        }

        let distinct_artists: HashSet<String> = pool
            .iter()
            .map(norm_artist)
            .filter(|a| !a.is_empty())
            .collect();
        if distinct_artists.len() < 2 {
            return pool;
        }

        let mut last_artist: Option<String> = existing_queue
            .last()
            .map(norm_artist)
            .filter(|a| !a.is_empty())
            .or_else(|| seed.map(norm_artist).filter(|a| !a.is_empty()));

        let mut remaining_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for t in &pool {
            let a = norm_artist(t);
            if !a.is_empty() {
                *remaining_counts.entry(a).or_insert(0) += 1;
            }
        }

        let mut out = Vec::with_capacity(pool.len());

        while !pool.is_empty() {
            let chosen_idx = pool
                .iter()
                .enumerate()
                .filter(|(_, t)| {
                    let a = norm_artist(t);
                    a.is_empty() || last_artist.as_deref() != Some(a.as_str())
                })
                .min_by_key(|(idx, t)| {
                    let a = norm_artist(t);
                    let rem = remaining_counts.get(&a).copied().unwrap_or(0);
                    (std::cmp::Reverse(rem), *idx)
                })
                .map(|(idx, _)| idx)
                .unwrap_or(0);

            let picked = pool.remove(chosen_idx);
            let artist = norm_artist(&picked);
            if !artist.is_empty() {
                if let Some(c) = remaining_counts.get_mut(&artist) {
                    *c = c.saturating_sub(1);
                }
                last_artist = Some(artist);
            }
            out.push(picked);
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str) -> Track {
        Track {
            id: id.to_owned(),
            title: format!("Title {id}"),
            artists: Vec::new(),
            album: None,
            duration_ms: 180_000,
            artwork_url: None,
            is_explicit: false,
            is_liked: false,
            is_video: false,
        }
    }

    #[test]
    fn reorder_and_remove_preserve_current_index() {
        let mut q = QueueState::default();
        QueueManager::replace(&mut q, vec![track("a"), track("b"), track("c")], 1).unwrap();
        QueueManager::reorder(&mut q, 2, 0);
        assert_eq!(q.items[0].id, "c");
        assert_eq!(q.current_index, Some(2));
        assert_eq!(q.current_track().unwrap().id, "b");

        QueueManager::remove(&mut q, 0);
        assert_eq!(q.current_index, Some(1));
        assert_eq!(q.current_track().unwrap().id, "b");
    }

    #[test]
    fn repeat_modes_and_radio_merge_dedup() {
        let mut q = QueueState::default();
        QueueManager::replace(&mut q, vec![track("a"), track("b")], 1).unwrap();
        assert_eq!(QueueManager::resolve_next_index(&q).unwrap(), None);
        assert_eq!(QueueManager::peek_upcoming_track_id(&q), None);

        QueueManager::cycle_repeat(&mut q);
        assert_eq!(q.repeat, RepeatMode::All);
        assert_eq!(QueueManager::resolve_next_index(&q).unwrap(), Some(0));
        assert_eq!(
            QueueManager::peek_upcoming_track_id(&q).as_deref(),
            Some("a")
        );

        let added_idx = QueueManager::merge_radio_tracks(&mut q, vec![track("a"), track("c")]);
        assert_eq!(added_idx, Some(2));
        assert_eq!(q.items.len(), 3);
    }

    #[test]
    fn diversify_radio_tracks_spreads_artists_and_dedups() {
        use resonance_contract::ArtistRef;
        let mk = |id: &str, title: &str, artist: &str| Track {
            id: id.to_owned(),
            title: title.to_owned(),
            artists: vec![ArtistRef {
                id: None,
                name: artist.to_owned(),
            }],
            album: None,
            duration_ms: 180_000,
            artwork_url: None,
            is_explicit: false,
            is_liked: false,
            is_video: false,
        };

        let seed = mk("s1", "Seed Song", "Artist A");
        let candidates = vec![
            mk("s1", "Seed Song", "Artist A"), // duplicate of seed
            mk("a1", "Song A1", "Artist A"),
            mk("a2", "Song A2", "Artist A"),
            mk("a3", "Song A3", "Artist A"),
            mk("b1", "Song B1", "Artist B"),
            mk("c1", "Song C1", "Artist C"),
            mk("d1", "Song D1", "Artist D"),
        ];

        let out = QueueManager::diversify_radio_tracks(Some(&seed), &[seed.clone()], candidates);
        assert_eq!(out.len(), 6);
        assert_ne!(out[0].artists[0].name, "Artist A");
        for pair in out.windows(2) {
            assert_ne!(
                pair[0].artists[0].name, pair[1].artists[0].name,
                "same artist should not appear back-to-back when alternatives exist"
            );
        }
    }
}
