//! W0 Foundation, Persistence & Contracts Offline Test Suite.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use resonance::live_engine::{
    load_cached_library_snapshot, load_persisted_settings, refresh_library_snapshot,
    save_persisted_settings,
};
use resonance::store::PlaybackSlice;
use resonance_contract::{
    CacheError, CacheStore, CoreError, CoreResult, HomeSection, LIBRARY_SNAPSHOT_METADATA_KEY,
    LyricsDocument, MusicProvider, OfflineQualityPreference, PersistedEqPreset, PersistedTheme,
    Playlist, SETTINGS_METADATA_KEY, SETTINGS_TTL_SECS, SearchResults, SettingsV1, StreamInfo,
    Track,
};
use resonance_core::cache::SqliteCacheStore;
use resonance_core::storage::{migrate_legacy_temp_dir_from, resolve_app_paths};
use tempfile::TempDir;

fn sample_tracks(count: usize) -> Vec<Track> {
    (0..count)
        .map(|i| Track {
            id: format!("vid_{i:02}"),
            title: format!("Track {i}"),
            artists: Vec::new(),
            album: None,
            duration_ms: 180_000,
            artwork_url: None,
            is_explicit: false,
            is_liked: false,
            is_video: false,
        })
        .collect()
}

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
                    id: "PL_W0_1".into(),
                    title: "W0 Saved Mix".into(),
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
fn app_paths_resolves_env_override_and_migrates_legacy_temp_idempotently() {
    let tmp = TempDir::new().unwrap();
    let data_override = tmp.path().join("w0_data");

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
        for dir in [
            &paths.data_dir,
            &paths.cache_dir,
            &paths.config_dir,
            &paths.downloads_dir,
        ] {
            let mode = fs::metadata(dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "dir {} must be 0o700", dir.display());
        }
    }

    // Legacy /tmp migration
    let fresh_root = tmp.path().join("fresh_persistent");
    let fresh_paths = resonance_contract::AppPaths {
        data_dir: fresh_root.clone(),
        cache_dir: fresh_root.join("cache"),
        config_dir: fresh_root.join("config"),
        downloads_dir: fresh_root.join("downloads"),
    };
    let legacy = tmp.path().join("legacy_tmp");
    fs::create_dir_all(legacy.join("downloads")).unwrap();
    fs::write(legacy.join("resonance.db"), b"legacy-db").unwrap();
    fs::write(legacy.join("resonance.db-wal"), b"legacy-wal").unwrap();
    fs::write(legacy.join("auth_session.json"), b"secret-auth").unwrap();
    fs::write(legacy.join("downloads/song.opus"), b"opus-bytes").unwrap();

    assert!(migrate_legacy_temp_dir_from(&legacy, &fresh_paths).unwrap());
    assert!(
        fresh_paths
            .data_dir
            .join(".migrated_from_temp_v1")
            .is_file()
    );
    assert_eq!(
        fs::read(fresh_paths.data_dir.join("resonance.db")).unwrap(),
        b"legacy-db"
    );
    assert_eq!(
        fs::read(fresh_paths.data_dir.join("resonance.db-wal")).unwrap(),
        b"legacy-wal"
    );
    assert_eq!(
        fs::read(fresh_paths.downloads_dir.join("song.opus")).unwrap(),
        b"opus-bytes"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let auth_mode = fs::metadata(fresh_paths.data_dir.join("auth_session.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(auth_mode, 0o600);
    }

    // Idempotent: second run is a no-op and never overwrites existing DB
    fs::write(fresh_paths.data_dir.join("resonance.db"), b"new-db").unwrap();
    assert!(!migrate_legacy_temp_dir_from(&legacy, &fresh_paths).unwrap());
    assert_eq!(
        fs::read(fresh_paths.data_dir.join("resonance.db")).unwrap(),
        b"new-db"
    );
}

#[test]
fn settings_v1_persists_across_restart_and_survives_clear_all_caches() {
    let tmp = TempDir::new().unwrap();
    let custom = SettingsV1 {
        schema_version: 1,
        theme: PersistedTheme::Dark,
        reduced_motion: true,
        sponsorblock_enabled: false,
        sponsorblock_categories: vec!["sponsor".into()],
        loudness_normalization: false,
        silence_skip_enabled: true,
        eq_preset: PersistedEqPreset::VocalBoost,
        eq_gains_db: [2.5; 10],
        crossfade_ms: 4000,
        playback_speed: 1.25,
        preserve_pitch: false,
        offline_mode: true,
        wifi_only_downloads: false,
        offline_quality: OfflineQualityPreference::High256,
        keep_liked_songs_offline: true,
        offline_quota_bytes: 2 * 1024 * 1024 * 1024,
        volume: 1.2,
        autoplay_radio: false,
        atr_watchtime_sync: false,
        update_channel: "beta".into(),
        local_music_dirs: vec!["/tmp/music".into()],
    };

    {
        let cache = SqliteCacheStore::open(tmp.path()).unwrap();
        save_persisted_settings(&cache, &custom);
        cache
            .put_metadata("home:sections", "{\"stale\":true}", 300, 1000)
            .unwrap();
        cache
            .put_metadata(
                LIBRARY_SNAPSHOT_METADATA_KEY,
                "{\"liked_songs\":[]}",
                300,
                1000,
            )
            .unwrap();
        cache.clear_all_caches().unwrap();
        assert!(cache.get_metadata("home:sections", 1001).unwrap().is_none());
        assert!(
            cache
                .get_metadata(LIBRARY_SNAPSHOT_METADATA_KEY, 1001)
                .unwrap()
                .is_some(),
            "library:% must survive clear_all_caches()"
        );
    }

    // Reopen DB (simulating restart)
    let reopened = SqliteCacheStore::open(tmp.path()).unwrap();
    let loaded = load_persisted_settings(&reopened);
    assert_eq!(loaded, custom);
}

#[test]
fn settings_v1_corrupt_json_recovers_to_defaults_without_panic() {
    let tmp = TempDir::new().unwrap();
    let cache = SqliteCacheStore::open(tmp.path()).unwrap();
    cache
        .put_metadata(
            SETTINGS_METADATA_KEY,
            "{\"schema_version\":1,\"theme\":\"dark\"",
            SETTINGS_TTL_SECS,
            resonance_core::unix_secs(),
        )
        .unwrap();

    let recovered = load_persisted_settings(&cache);
    assert_eq!(recovered, SettingsV1::default());
}

#[test]
fn shuffle_deck_produces_true_permutation_keeps_current_track_and_restores_order() {
    let tracks = sample_tracks(20);
    let mut slice = PlaybackSlice::new(true, tracks.clone(), |id| format!("http://127.0.0.1/{id}"));

    // Jump to index 7 before enabling shuffle
    slice.jump_queue(7, false, |id| format!("http://127.0.0.1/{id}"));
    assert_eq!(slice.player.queue.current_index, Some(7));

    slice.toggle_shuffle();
    assert!(slice.player.queue.shuffle);
    assert_eq!(slice.shuffle_deck.order.len(), 20);
    assert_eq!(slice.shuffle_deck.cursor, 0);
    assert_eq!(
        slice.shuffle_deck.order[0], 7,
        "current track must stay at order[0]"
    );

    // Verify true permutation of 0..20 and non-reversal
    let unique: HashSet<usize> = slice.shuffle_deck.order.iter().copied().collect();
    assert_eq!(unique.len(), 20);
    let reversed: Vec<usize> = (0..20).rev().collect();
    assert_ne!(slice.shuffle_deck.order, reversed);

    // Play Next inserts directly at cursor + 1
    let inserted = sample_tracks(1).remove(0);
    slice.play_next(Track {
        id: "inserted_next".into(),
        ..inserted
    });
    let next_idx = slice.shuffle_deck.order[slice.shuffle_deck.cursor + 1];
    assert_eq!(slice.player.queue.items[next_idx].id, "inserted_next");

    // Advance one track and unshuffle -> restores natural queue index of current track
    let advanced = slice
        .next_track(false, |id| format!("http://127.0.0.1/{id}"))
        .unwrap();
    assert_eq!(advanced.id, "inserted_next");
    slice.toggle_shuffle();
    assert!(!slice.player.queue.shuffle);
    assert_eq!(slice.player.queue.current_index, Some(next_idx));
    assert!(slice.shuffle_deck.order.is_empty());
}

#[tokio::test]
async fn library_snapshot_hydrates_offline_and_clears_on_sign_out() {
    let tmp = TempDir::new().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(tmp.path()).unwrap());
    let tracks = sample_tracks(2);
    let provider = MockLibraryProvider {
        liked: vec![tracks[0].clone()],
    };

    assert!(load_cached_library_snapshot(cache.as_ref()).is_none());
    let snap = refresh_library_snapshot(&provider, cache.as_ref()).await;
    assert_eq!(snap.liked_songs.len(), 1);
    assert_eq!(snap.playlists.len(), 1);

    // Offline reload
    let hydrated = load_cached_library_snapshot(cache.as_ref()).unwrap();
    assert_eq!(hydrated.liked_songs[0].id, tracks[0].id);

    // Sign-out privacy wipe
    cache.evict_metadata(LIBRARY_SNAPSHOT_METADATA_KEY).unwrap();
    assert!(load_cached_library_snapshot(cache.as_ref()).is_none());
}

#[test]
fn nsh_version_and_no_openssl_android_gate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cargo_toml = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(cargo_toml.contains("nsh-core = \"=0.7.0\""));
    assert!(cargo_toml.contains("nsh-base = { version = \"=0.7.0\""));
    assert!(cargo_toml.contains("nsh-engine = { version = \"=0.7.0\""));
    assert!(cargo_toml.contains("nsh-testing = { version = \"=0.7.0\""));
    assert!(!cargo_toml.contains("openssl"));

    let arch_md = fs::read_to_string(root.join("ARCHITECTURE.md")).unwrap();
    assert!(arch_md.contains("=0.7.0"));
}
