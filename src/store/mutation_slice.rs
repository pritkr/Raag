//! Optimistic account-mutation slice (`MutationSlice`) handling like/dislike
//! and playlist edit transitions with deterministic rollbacks on `LiveUpdate::MutationFailed`.

use resonance_contract::{LikeAction, MutationCommand, PlaylistEditOp, Track, TrackId};

use super::catalog_slice::CatalogSlice;
use super::library_slice::LibrarySlice;
use super::playback_slice::PlaybackSlice;

#[derive(Clone)]
pub struct PendingPlaylistEdit {
    pub playlist_id: String,
    pub previous_tracks: Vec<Track>,
    pub previous_count: Option<u32>,
}

#[derive(Clone, Default)]
pub struct MutationSlice {
    pub pending_mutations: usize,
    pub pending_playlist_edits: Vec<PendingPlaylistEdit>,
}

impl MutationSlice {
    pub fn toggle_like_current(
        &mut self,
        playback: &mut PlaybackSlice,
        library: &mut LibrarySlice,
        live: bool,
        signed_in: bool,
    ) -> Option<(TrackId, LikeAction)> {
        let cur = playback.player.current_track.as_mut()?;
        cur.is_liked = !cur.is_liked;
        if cur.is_liked {
            library.disliked_ids.retain(|id| id != &cur.id);
            if !library.liked_tracks.iter().any(|t| t.id == cur.id) {
                library.liked_tracks.insert(0, cur.clone());
            }
            if live && signed_in {
                self.pending_mutations += 1;
                return Some((cur.id.clone(), LikeAction::Like));
            }
        } else {
            library.liked_tracks.retain(|t| t.id != cur.id);
            if live && signed_in {
                self.pending_mutations += 1;
                return Some((cur.id.clone(), LikeAction::RemoveLike));
            }
        }
        None
    }

    pub fn toggle_like_track(
        &mut self,
        mut track: Track,
        playback: &mut PlaybackSlice,
        library: &mut LibrarySlice,
        live: bool,
        signed_in: bool,
    ) -> Option<(TrackId, LikeAction)> {
        let already = library.liked_tracks.iter().any(|t| t.id == track.id);
        let next_liked = !already;
        if next_liked {
            track.is_liked = true;
            library.disliked_ids.retain(|id| id != &track.id);
            library.liked_tracks.insert(0, track.clone());
        } else {
            library.liked_tracks.retain(|t| t.id != track.id);
        }
        if let Some(cur) = playback.player.current_track.as_mut()
            && cur.id == track.id
        {
            cur.is_liked = next_liked;
        }
        if live && signed_in {
            self.pending_mutations += 1;
            let action = if next_liked {
                LikeAction::Like
            } else {
                LikeAction::RemoveLike
            };
            return Some((track.id, action));
        }
        None
    }

    pub fn toggle_dislike_current(
        &mut self,
        playback: &mut PlaybackSlice,
        library: &mut LibrarySlice,
        live: bool,
        signed_in: bool,
    ) -> (Option<(TrackId, LikeAction)>, Option<String>) {
        let Some(cur) = playback.player.current_track.as_mut() else {
            return (None, None);
        };
        if let Some(pos) = library.disliked_ids.iter().position(|id| id == &cur.id) {
            library.disliked_ids.remove(pos);
            let dispatch = if live && signed_in {
                self.pending_mutations += 1;
                Some((cur.id.clone(), LikeAction::RemoveLike))
            } else {
                None
            };
            (dispatch, None)
        } else {
            library.disliked_ids.push(cur.id.clone());
            cur.is_liked = false;
            library.liked_tracks.retain(|t| t.id != cur.id);
            let banner = Some(format!("Less like \"{}\"", cur.title));
            let dispatch = if live && signed_in {
                self.pending_mutations += 1;
                Some((cur.id.clone(), LikeAction::Dislike))
            } else {
                None
            };
            (dispatch, banner)
        }
    }

    pub fn apply_playlist_edit(
        &mut self,
        playlist_id: &str,
        op: &PlaylistEditOp,
        track_to_add: Option<Track>,
        catalog: &mut CatalogSlice,
        library: &mut LibrarySlice,
        live: bool,
        signed_in: bool,
    ) -> Option<(String, PlaylistEditOp)> {
        let previous_tracks = catalog
            .selected_collection
            .as_ref()
            .filter(|c| c.id == playlist_id)
            .map(|c| c.tracks.clone())
            .unwrap_or_default();
        let previous_count = library
            .playlists
            .iter()
            .find(|p| p.playlist_id.as_deref() == Some(playlist_id) || p.browse_id == playlist_id)
            .and_then(|p| p.track_count);

        if live && signed_in {
            self.pending_mutations += 1;
            self.pending_playlist_edits.push(PendingPlaylistEdit {
                playlist_id: playlist_id.to_owned(),
                previous_tracks,
                previous_count,
            });
            if self.pending_playlist_edits.len() > 32 {
                self.pending_playlist_edits.remove(0);
            }
        }

        if let Some(col) = catalog.selected_collection.as_mut()
            && col.id == playlist_id
        {
            match op {
                PlaylistEditOp::AddVideo { video_id } => {
                    if let Some(t) = track_to_add {
                        col.tracks.push(t);
                    } else if !col.tracks.iter().any(|t| t.id == *video_id) {
                        col.tracks.push(Track {
                            id: video_id.clone(),
                            title: video_id.clone(),
                            artists: Vec::new(),
                            album: None,
                            duration_ms: 0,
                            artwork_url: None,
                            is_explicit: false,
                            is_liked: false,
                            is_video: false,
                        });
                    }
                }
                PlaylistEditOp::RemoveVideo { video_id, .. } => {
                    col.tracks.retain(|t| t.id != *video_id);
                }
                PlaylistEditOp::MoveVideoBefore {
                    set_video_id,
                    predecessor_set_video_id,
                } => {
                    if let Some(from_idx) = col.tracks.iter().position(|t| t.id == *set_video_id) {
                        let item = col.tracks.remove(from_idx);
                        let target_idx = match predecessor_set_video_id {
                            Some(pred_id) => col
                                .tracks
                                .iter()
                                .position(|t| t.id == *pred_id)
                                .unwrap_or(col.tracks.len()),
                            None => 0,
                        };
                        col.tracks.insert(target_idx, item);
                    }
                }
            }
        }

        if let Some(pl) = library
            .playlists
            .iter_mut()
            .find(|p| p.playlist_id.as_deref() == Some(playlist_id) || p.browse_id == playlist_id)
        {
            match op {
                PlaylistEditOp::AddVideo { .. } => {
                    pl.track_count = Some(pl.track_count.unwrap_or(0).saturating_add(1));
                }
                PlaylistEditOp::RemoveVideo { .. } => {
                    pl.track_count = Some(pl.track_count.unwrap_or(1).saturating_sub(1));
                }
                PlaylistEditOp::MoveVideoBefore { .. } => {}
            }
        }

        if live && signed_in {
            Some((playlist_id.to_owned(), op.clone()))
        } else {
            None
        }
    }

    pub fn rollback_mutation(
        &mut self,
        command: MutationCommand,
        playback: &mut PlaybackSlice,
        library: &mut LibrarySlice,
        catalog: &mut CatalogSlice,
    ) {
        self.pending_mutations = self.pending_mutations.saturating_sub(1);
        match command {
            MutationCommand::SetLike { video_id, action } => match action {
                LikeAction::Like => {
                    library.liked_tracks.retain(|t| t.id != video_id);
                    if let Some(cur) = playback.player.current_track.as_mut()
                        && cur.id == video_id
                    {
                        cur.is_liked = false;
                    }
                }
                LikeAction::RemoveLike => {
                    if let Some(cur) = playback.player.current_track.as_mut()
                        && cur.id == video_id
                    {
                        cur.is_liked = true;
                        if !library.liked_tracks.iter().any(|t| t.id == video_id) {
                            library.liked_tracks.insert(0, cur.clone());
                        }
                    }
                }
                LikeAction::Dislike => {
                    library.disliked_ids.retain(|id| id != &video_id);
                }
            },
            MutationCommand::EditPlaylist { playlist_id, .. } => {
                if let Some(pos) = self
                    .pending_playlist_edits
                    .iter()
                    .rposition(|e| e.playlist_id == playlist_id)
                {
                    let snapshot = self.pending_playlist_edits.remove(pos);
                    if let Some(col) = catalog.selected_collection.as_mut()
                        && col.id == playlist_id
                    {
                        col.tracks = snapshot.previous_tracks;
                    }
                    if let Some(pl) = library.playlists.iter_mut().find(|p| {
                        p.playlist_id.as_deref() == Some(playlist_id.as_str())
                            || p.browse_id == playlist_id
                    }) {
                        pl.track_count = snapshot.previous_count;
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use resonance_core::fixture::fixture_tracks;

    #[test]
    fn optimistic_like_and_rollback_stay_consistent() {
        let tracks = fixture_tracks();
        let mut playback =
            PlaybackSlice::new(true, tracks.clone(), |id| format!("http://127.0.0.1/{id}"));
        let mut library = LibrarySlice::new(true, Vec::new());
        let mut catalog = CatalogSlice::new(true);
        let mut mutations = MutationSlice::default();

        if let Some(cur) = playback.player.current_track.as_mut() {
            cur.is_liked = false;
        }
        let dispatched = mutations.toggle_like_current(&mut playback, &mut library, true, true);
        assert!(matches!(dispatched, Some((_, LikeAction::Like))));
        assert!(playback.player.current_track.as_ref().unwrap().is_liked);
        assert_eq!(mutations.pending_mutations, 1);

        let vid = playback.player.current_track.as_ref().unwrap().id.clone();
        mutations.rollback_mutation(
            MutationCommand::SetLike {
                video_id: vid,
                action: LikeAction::Like,
            },
            &mut playback,
            &mut library,
            &mut catalog,
        );
        assert!(!playback.player.current_track.as_ref().unwrap().is_liked);
        assert!(library.liked_tracks.is_empty());
        assert_eq!(mutations.pending_mutations, 0);
    }
}
