//! Optimistic account-mutation slice (`MutationSlice`) handling like/dislike
//! transitions and deterministic rollbacks on `LiveUpdate::MutationFailed`.

use resonance_contract::{LikeAction, MutationCommand, Track, TrackId};

use super::library_slice::LibrarySlice;
use super::playback_slice::PlaybackSlice;

#[derive(Clone, Default)]
pub struct MutationSlice {
    pub pending_mutations: usize,
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

    pub fn rollback_mutation(
        &mut self,
        command: MutationCommand,
        playback: &mut PlaybackSlice,
        library: &mut LibrarySlice,
    ) {
        self.pending_mutations = self.pending_mutations.saturating_sub(1);
        if let MutationCommand::SetLike { video_id, action } = command {
            match action {
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
            }
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
        let mut playback = PlaybackSlice::new(true, tracks.clone(), |id| format!("http://127.0.0.1/{id}"));
        let mut library = LibrarySlice::new(true, Vec::new());
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
        );
        assert!(!playback.player.current_track.as_ref().unwrap().is_liked);
        assert!(library.liked_tracks.is_empty());
        assert_eq!(mutations.pending_mutations, 0);
    }
}
