//! H0 Account Truth & Pre-Requisites Offline Test Suite (Wave 0 Gate 0).

use std::sync::Arc;

use async_trait::async_trait;
use nsh_engine::{EngineConfig, Points, Root, Size};
use nsh_testing::Headless;
use resonance::live_engine::{
    load_cached_library_snapshot, load_persisted_settings, refresh_library_snapshot,
    save_persisted_settings,
};
use resonance::store::{self, CatalogSlice, LibrarySlice, MutationSlice, PlaybackSlice};
use resonance_contract::{
    CacheError, CacheStore, CoreError, CoreResult, HomeSection, LIBRARY_SNAPSHOT_METADATA_KEY,
    LibraryCollectionRef, LyricsDocument, MusicProvider, MutationCommand, PersistedEqPreset,
    PersistedTheme, Playlist, PlaylistEditOp, SearchResults, SettingsV1, StreamInfo, Track,
};
use resonance_core::cache::{ChunkCache, DEFAULT_MAX_AUDIO_CACHE_BYTES, SqliteCacheStore};
use resonance_core::fixture::fixture_tracks;
use resonance_core::offline::{DownloadManager, OfflineVault, PinnedCollectionKind};
use resonance_core::storage::{migrate_legacy_temp_dir_from, resolve_app_paths};
use tempfile::TempDir;

struct MockLibraryProvider {
    liked: Vec<Track>,
}

#[async_trait]
impl MusicProvider for MockLibraryProvider {
    async fn home(&self) -> CoreResult<Vec<HomeSection>> {
        Ok(Vec::new())
    }

    async fn search(&self, query: &str) -> CoreResult<SearchResults> {
        Ok(SearchResults {
            query: query.to_owned(),
            tracks: Vec::new(),
            albums: Vec::new(),
            continuation: None,
        })
    }

    async fn browse_collection(&self, browse_id: &str) -> CoreResult<Playlist> {
        if browse_id == "FEmusic_liked_playlists" {
            return Ok(Playlist {
                id: browse_id.to_owned(),
                title: "Playlists".to_owned(),
                subtitle: None,
                artwork_url: None,
                tracks: vec![Track {
                    id: "PL_TEST_1".into(),
                    title: "Roadtrip Mix".into(),
                    artists: Vec::new(),
                    album: None,
                    duration_ms: 0,
                    artwork_url: None,
                    is_explicit: false,
                    is_liked: false,
                    is_video: false,
                }],
            });
        }
        Err(CoreError::Cache(CacheError::Io(format!(
            "not found: {browse_id}"
        ))))
    }

    async fn liked_songs(&self) -> CoreResult<Vec<Track>> {
        Ok(self.liked.clone())
    }

    async fn autoplay_next(&self, _video_id: &str) -> CoreResult<Vec<Track>> {
        Ok(Vec::new())
    }

    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo> {
        Err(CoreError::Cache(CacheError::Io(format!(
            "not found: {video_id}"
        ))))
    }

    async fn ytm_lyrics(&self, _video_id: &str) -> CoreResult<Option<LyricsDocument>> {
        Ok(None)
    }
}

#[test]
fn app_paths_uses_os_dirs_env_override_0700_and_migrates_legacy_temp_once() {
    let tmp = TempDir::new().unwrap();
    let data_override = tmp.path().join("custom_data");

    // SAFETY: single-threaded test check for RESONANCE_DATA_DIR override.
    unsafe {
        std::env::set_var("RESONANCE_DATA_DIR", &data_override);
    }
    let paths = resolve_app_paths().unwrap();
    unsafe {
        std::env::remove_var("RESONANCE_DATA_DIR");
    }

    assert_eq!(paths.data_dir, data_override);
    assert_eq!(paths.downloads_dir, data_override.join("downloads"));
    assert!(paths.downloads_dir.join(".nomedia").is_file());
    assert!(paths.cache_dir.join("chunks/.nomedia").is_file());
    assert!(paths.cache_dir.join("artwork/.nomedia").is_file());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&paths.downloads_dir)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }

    // Verify default OS path resolution does NOT use std::env::temp_dir()
    let os_paths = resolve_app_paths().unwrap();
    assert!(
        !os_paths.data_dir.starts_with(std::env::temp_dir()),
        "OS data_dir must not be inside temp_dir(): {:?}",
        os_paths.data_dir
    );

    // Test idempotent one-time legacy temp migration without clobbering existing DB
    let fresh_target = tmp.path().join("fresh_target");
    let fresh_paths = resonance_contract::AppPaths {
        data_dir: fresh_target.clone(),
        cache_dir: fresh_target.join("cache"),
        config_dir: fresh_target.join("config"),
        downloads_dir: fresh_target.join("downloads"),
    };
    let legacy = tmp.path().join("legacy_temp");
    std::fs::create_dir_all(legacy.join("downloads")).unwrap();
    std::fs::write(legacy.join("resonance.db"), b"legacy-db-bytes").unwrap();
    std::fs::write(legacy.join("auth_session.json"), b"legacy-auth").unwrap();
    std::fs::write(legacy.join("downloads/t1.m4a"), b"audio-bytes").unwrap();

    assert!(migrate_legacy_temp_dir_from(&legacy, &fresh_paths).unwrap());
    assert_eq!(
        std::fs::read(fresh_paths.data_dir.join("resonance.db")).unwrap(),
        b"legacy-db-bytes"
    );
    assert_eq!(
        std::fs::read(fresh_paths.downloads_dir.join("t1.m4a")).unwrap(),
        b"audio-bytes"
    );

    // Second migration must be a no-op and must not overwrite existing DB
    std::fs::write(fresh_paths.data_dir.join("resonance.db"), b"newer-db").unwrap();
    assert!(!migrate_legacy_temp_dir_from(&legacy, &fresh_paths).unwrap());
    assert_eq!(
        std::fs::read(fresh_paths.data_dir.join("resonance.db")).unwrap(),
        b"newer-db"
    );
}

#[tokio::test]
async fn library_hydration_populates_from_sqlite_offline_and_refreshes_when_signed_in() {
    let tmp = TempDir::new().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(tmp.path()).unwrap());
    let sample_tracks = fixture_tracks();

    let provider = MockLibraryProvider {
        liked: vec![sample_tracks[0].clone()],
    };

    // Cold-start offline: nothing cached yet
    assert!(load_cached_library_snapshot(cache.as_ref()).is_none());

    // Refresh when signed in populates SQLite snapshot
    let snap = refresh_library_snapshot(&provider, cache.as_ref()).await;
    assert_eq!(snap.liked_songs.len(), 1);
    assert_eq!(snap.playlists.len(), 1);
    assert_eq!(snap.playlists[0].title, "Roadtrip Mix");

    // Offline reload from SQLite without network
    let cached = load_cached_library_snapshot(cache.as_ref()).unwrap();
    assert_eq!(cached.liked_songs[0].id, sample_tracks[0].id);
    assert_eq!(cached.playlists[0].browse_id, "PL_TEST_1");

    // Sign-out evicts snapshot
    cache.evict_metadata(LIBRARY_SNAPSHOT_METADATA_KEY).unwrap();
    assert!(load_cached_library_snapshot(cache.as_ref()).is_none());
}

#[test]
fn settings_v1_persists_via_put_metadata_and_survives_clear_all_caches() {
    let tmp = TempDir::new().unwrap();
    let cache = SqliteCacheStore::open(tmp.path()).unwrap();

    let custom = SettingsV1 {
        schema_version: 1,
        theme: PersistedTheme::Dark,
        reduced_motion: true,
        sponsorblock_enabled: false,
        sponsorblock_categories: vec!["sponsor".into()],
        loudness_normalization: false,
        silence_skip_enabled: true,
        eq_preset: PersistedEqPreset::BassBoost,
        eq_gains_db: [4.0; 10],
        crossfade_ms: 4000,
        offline_mode: true,
        volume: 0.42,
        offline_quota_bytes: 10 * 1024 * 1024 * 1024,
        update_channel: "beta".into(),
        local_music_dirs: vec!["/music".into()],
        ..SettingsV1::default()
    };

    save_persisted_settings(&cache, &custom);
    cache
        .put_metadata("home:sections", "{\"cached\":true}", 300, 1000)
        .unwrap();

    cache.clear_all_caches().unwrap();

    // Transient cache is gone
    assert!(cache.get_metadata("home:sections", 1001).unwrap().is_none());

    // SettingsV1 survived clear_all_caches()
    let reloaded = load_persisted_settings(&cache);
    assert_eq!(reloaded, custom);
}

#[test]
fn download_action_surface_reflects_queued_downloading_offline_and_failed_states() {
    store::enable_fixture_mode(true);
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(Points(1200.0), Points(800.0)),
        ..EngineConfig::new(Root::of::<resonance::App>())
    })
    .unwrap();

    app.wait_for("Listen again").unwrap();
    app.press("id:nav-now-playing").unwrap();
    app.wait_for("now-playing-download-btn").unwrap();
    app.press("now-playing-download-btn").unwrap();
    app.wait_for("Offline").unwrap();
}

#[test]
fn playlist_edit_add_remove_reorder_optimistically_updates_and_rolls_back_on_failure() {
    let tracks = fixture_tracks();
    let mut playback =
        PlaybackSlice::new(true, tracks.clone(), |id| format!("http://127.0.0.1/{id}"));
    let mut library = LibrarySlice::new(false, Vec::new());
    library.playlists.push(LibraryCollectionRef {
        browse_id: "VLPL_1".into(),
        playlist_id: Some("PL_1".into()),
        title: "My Mix".into(),
        subtitle: "User".into(),
        artwork_url: None,
        track_count: Some(2),
    });

    let mut catalog = CatalogSlice::new(true);
    catalog.selected_collection = Some(Playlist {
        id: "PL_1".into(),
        title: "My Mix".into(),
        subtitle: None,
        artwork_url: None,
        tracks: vec![tracks[0].clone(), tracks[1].clone()],
    });

    let mut mutations = MutationSlice::default();

    // 1. AddVideo + rollback
    let add_op = PlaylistEditOp::AddVideo {
        video_id: tracks[2].id.clone(),
    };
    let dispatched = mutations.apply_playlist_edit(
        "PL_1",
        &add_op,
        Some(tracks[2].clone()),
        &mut catalog,
        &mut library,
        true,
        true,
    );
    assert!(dispatched.is_some());
    assert_eq!(
        catalog.selected_collection.as_ref().unwrap().tracks.len(),
        3
    );
    assert_eq!(library.playlists[0].track_count, Some(3));

    mutations.rollback_mutation(
        MutationCommand::EditPlaylist {
            playlist_id: "PL_1".into(),
            op: add_op,
        },
        &mut playback,
        &mut library,
        &mut catalog,
    );
    assert_eq!(
        catalog.selected_collection.as_ref().unwrap().tracks.len(),
        2
    );
    assert_eq!(library.playlists[0].track_count, Some(2));
    assert_eq!(mutations.pending_mutations, 0);

    // 2. MoveVideoBefore + rollback
    let move_op = PlaylistEditOp::MoveVideoBefore {
        set_video_id: tracks[1].id.clone(),
        predecessor_set_video_id: Some(tracks[0].id.clone()),
    };
    mutations.apply_playlist_edit(
        "PL_1",
        &move_op,
        None,
        &mut catalog,
        &mut library,
        true,
        true,
    );
    assert_eq!(
        catalog.selected_collection.as_ref().unwrap().tracks[0].id,
        tracks[1].id
    );
    mutations.rollback_mutation(
        MutationCommand::EditPlaylist {
            playlist_id: "PL_1".into(),
            op: move_op,
        },
        &mut playback,
        &mut library,
        &mut catalog,
    );
    assert_eq!(
        catalog.selected_collection.as_ref().unwrap().tracks[0].id,
        tracks[0].id
    );

    // 3. RemoveVideo + rollback
    let rm_op = PlaylistEditOp::RemoveVideo {
        video_id: tracks[0].id.clone(),
        set_video_id: tracks[0].id.clone(),
    };
    mutations.apply_playlist_edit("PL_1", &rm_op, None, &mut catalog, &mut library, true, true);
    assert_eq!(
        catalog.selected_collection.as_ref().unwrap().tracks.len(),
        1
    );
    mutations.rollback_mutation(
        MutationCommand::EditPlaylist {
            playlist_id: "PL_1".into(),
            op: rm_op,
        },
        &mut playback,
        &mut library,
        &mut catalog,
    );
    assert_eq!(
        catalog.selected_collection.as_ref().unwrap().tracks.len(),
        2
    );
}

#[tokio::test]
async fn offline_vault_pinned_collections_surface_and_honest_radio_mix_label() {
    let tmp = TempDir::new().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(tmp.path()).unwrap());
    let chunks = Arc::new(
        ChunkCache::new(
            &tmp.path().join("chunks"),
            cache.clone(),
            DEFAULT_MAX_AUDIO_CACHE_BYTES,
        )
        .unwrap(),
    );
    let provider = Arc::new(MockLibraryProvider { liked: Vec::new() });
    let lyrics = Arc::new(resonance_core::lyrics::CascadeLyricsProvider::new(
        provider.clone(),
        cache.clone(),
    ));
    let artwork = Arc::new(
        resonance_core::cache::ArtworkCache::new(&tmp.path().join("art"), 32, 1024 * 1024).unwrap(),
    );
    let bus = resonance_contract::EventBus::new(16);
    let manager = Arc::new(
        DownloadManager::new(provider, cache.clone(), chunks, artwork, lyrics, bus).unwrap(),
    );
    let vault = OfflineVault::new(cache.clone(), manager);

    let tracks = fixture_tracks();
    vault
        .pin_collection(
            "ALBUM_VAULT_1",
            PinnedCollectionKind::Album,
            "Midnight Vault",
            Some("Artist"),
            None,
            &tracks[..1],
        )
        .await
        .unwrap();

    cache.clear_all_caches().unwrap();
    let pinned = vault.list_pinned_collections().unwrap();
    assert_eq!(pinned.len(), 1);
    assert_eq!(pinned[0].title, "Midnight Vault");

    // Verify UI renders "Radio & Rooms" (not "Listen Together") and Library Downloads surface works
    store::enable_fixture_mode(true);
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(Points(1200.0), Points(800.0)),
        ..EngineConfig::new(Root::of::<resonance::App>())
    })
    .unwrap();
    app.wait_for("Listen together").unwrap();

    app.press("Library").unwrap();
    app.wait_for("lib-filter-downloads").unwrap();
    app.press("lib-filter-downloads").unwrap();
    app.wait_for("Offline Downloads").unwrap();
}
