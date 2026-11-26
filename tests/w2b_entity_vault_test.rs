//! Deterministic offline verification for W2B — Dedicated Artist & Album Entity Screens
//! and First-Class Offline Vault (ref-counted unpin, quota/Wi-Fi guard, SHA-256 audit).

use std::fs;
use std::path::Path;
use std::sync::Arc;

use nsh_core::prelude::{IntoElement, ParentElement, Styled, component, view};
use nsh_engine::{ColorScheme, EngineConfig, Environment, Points, Root, Size};
use nsh_testing::Headless;
use resonance::screens::album::{AlbumScreenCallbacks, album_screen_with_detail};
use resonance::screens::artist::{ArtistScreenCallbacks, artist_screen_with_detail};
use resonance_contract::{
    AlbumDetail, ArtistDetail, ArtistRef, DownloadStatus, LibraryCollectionRef, OfflineStore, Track,
};
use resonance_core::cache::SqliteCacheStore;
use resonance_core::downloads::CollectionOfflineProgress;
use resonance_core::downloads::quota::{QuotaAdmission, VaultQuotaGuard};
use resonance_core::downloads::vault::{OfflineVault, PinnedCollectionKind};
use resonance_core::offline::DownloadManager;
use tempfile::TempDir;

fn sample_track(id: &str, title: &str) -> Track {
    Track {
        id: id.to_owned(),
        title: title.to_owned(),
        artists: vec![ArtistRef {
            id: Some("UC_ART_1".to_owned()),
            name: "Kaelen Vance".to_owned(),
        }],
        album: None,
        duration_ms: 210_000,
        artwork_url: None,
        is_explicit: false,
        is_liked: false,
        is_video: false,
    }
}

fn make_test_vault(dir: &Path) -> (Arc<SqliteCacheStore>, OfflineVault) {
    let db = Arc::new(SqliteCacheStore::open(dir).unwrap());
    let chunks = Arc::new(
        resonance_core::cache::ChunkCache::new(
            &dir.join("chunks"),
            db.clone(),
            resonance_core::cache::DEFAULT_MAX_AUDIO_CACHE_BYTES,
        )
        .unwrap(),
    );
    let provider: Arc<dyn resonance_contract::MusicProvider> = Arc::new(
        resonance_core::fixture::FixtureProvider::new("http://127.0.0.1:0/fixture.opus"),
    );
    let lyrics: Arc<dyn resonance_contract::LyricsProvider> = Arc::new(
        resonance_core::lyrics::CascadeLyricsProvider::new(provider.clone(), db.clone()),
    );
    let artwork = Arc::new(
        resonance_core::cache::ArtworkCache::new(&dir.join("art"), 32, 1024 * 1024).unwrap(),
    );
    let bus = resonance_contract::EventBus::new(16);
    let offline_store: Arc<dyn OfflineStore> = db.clone();
    let mgr = Arc::new(
        DownloadManager::new(provider, offline_store, chunks, artwork, lyrics, bus).unwrap(),
    );
    let vault = OfflineVault::new(db.clone(), mgr);
    (db, vault)
}

#[tokio::test]
async fn ref_counted_unpin_retains_shared_tracks_and_deletes_orphans() {
    let dir = TempDir::new().unwrap();
    let (db, vault) = make_test_vault(dir.path());

    let t1 = sample_track("t1", "Track 1");
    let t2 = sample_track("t2", "Track 2 (Shared)");
    let t3 = sample_track("t3", "Track 3");
    let t4 = sample_track("t4", "Track 4");

    vault
        .pin_collection(
            "ALB_A",
            PinnedCollectionKind::Album,
            "Album A",
            None,
            None,
            &[t1.clone(), t2.clone(), t3.clone()],
        )
        .await
        .unwrap();
    vault
        .pin_collection(
            "PL_B",
            PinnedCollectionKind::Playlist,
            "Playlist B",
            None,
            None,
            &[t2.clone(), t4.clone()],
        )
        .await
        .unwrap();

    // Commit synthetic media files for all 4 tracks.
    for t in [&t1, &t2, &t3, &t4] {
        let rel = format!("{}.m4a", t.id);
        let full = db.downloads_root().join(&rel);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(&full, format!("audio-bytes-{}", t.id)).unwrap();
        db.mark_download_completed(&t.id, &rel, 128, "deadbeef", true, 1000)
            .unwrap();
    }

    let report = vault.unpin_collection_with_cleanup("ALB_A").await.unwrap();
    assert_eq!(report.unpinned_id, "ALB_A");
    assert_eq!(report.deleted_tracks, 2);
    assert_eq!(report.retained_shared_tracks, 1);

    assert!(db.get_download("t1").unwrap().is_none());
    assert!(db.get_download("t3").unwrap().is_none());
    assert!(db.get_download("t2").unwrap().is_some());
    assert!(db.get_download("t4").unwrap().is_some());
}

#[test]
fn quota_guard_blocks_over_budget_and_defers_on_metered_wifi_only() {
    let guard = VaultQuotaGuard {
        quota_bytes: 10_000_000,
        wifi_only: true,
        is_metered_network: false,
    };
    assert_eq!(
        guard.evaluate(6_000_000, 3_000_000),
        QuotaAdmission::Allowed {
            remaining_bytes: 1_000_000
        }
    );
    assert_eq!(
        guard.evaluate(6_000_000, 5_000_000),
        QuotaAdmission::BlockedQuotaExceeded {
            used_bytes: 6_000_000,
            quota_bytes: 10_000_000,
            needed_bytes: 5_000_000,
        }
    );

    let metered = VaultQuotaGuard {
        quota_bytes: 0,
        wifi_only: true,
        is_metered_network: true,
    };
    assert_eq!(
        metered.evaluate(0, 1_000_000),
        QuotaAdmission::DeferredMeteredNetwork
    );

    let unlimited = VaultQuotaGuard {
        quota_bytes: 0,
        wifi_only: false,
        is_metered_network: true,
    };
    assert_eq!(
        unlimited.evaluate(50_000_000_000, 10_000_000),
        QuotaAdmission::Allowed {
            remaining_bytes: u64::MAX
        }
    );
}

#[tokio::test]
async fn vault_integrity_audit_detects_bitrot_and_marks_repairable() {
    let dir = TempDir::new().unwrap();
    let (db, vault) = make_test_vault(dir.path());

    let t1 = sample_track("bitrot1", "Bitrot Track");
    vault.enqueue_track(t1.clone()).await.unwrap();

    let rel = "bitrot1.m4a";
    let full = db.downloads_root().join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    let payload = b"clean-audio-frame-data";
    fs::write(&full, payload).unwrap();

    use sha2::{Digest, Sha256};
    let good_sha = hex::encode(Sha256::digest(payload));
    db.mark_download_completed("bitrot1", rel, payload.len() as u64, &good_sha, true, 1000)
        .unwrap();

    let clean = vault.audit_all_tracks().unwrap();
    assert_eq!(clean.verified_count, 1);
    assert!(clean.corrupted_video_ids.is_empty());

    // Flip 1 byte on disk to simulate bitrot.
    let mut corrupted = payload.to_vec();
    corrupted[0] ^= 0xFF;
    fs::write(&full, &corrupted).unwrap();

    let audit = vault.audit_all_tracks().unwrap();
    assert_eq!(audit.verified_count, 0);
    assert_eq!(audit.corrupted_video_ids, vec!["bitrot1".to_owned()]);

    let rec = db.get_download("bitrot1").unwrap().unwrap();
    assert_eq!(rec.status, DownloadStatus::Failed);
}

#[test]
fn artist_and_album_screens_render_and_have_zero_raw_color_literals() {
    for path in [
        "src/screens/artist.rs",
        "src/screens/album.rs",
        "src/ui/controls/vault_widgets.rs",
    ] {
        let src = fs::read_to_string(Path::new(path)).expect("read W2B UI file");
        for needle in ["rgb(", "rgba(", "0xFF"] {
            assert!(
                !src.contains(needle),
                "raw color literal `{needle}` found in {path}"
            );
        }
    }

    #[component]
    fn EntityScreensRoot() -> impl IntoElement {
        let artist = ArtistDetail {
            channel_id: "UC_ART_1".to_owned(),
            name: "Kaelen Vance".to_owned(),
            description: Some("Ambient & electronic composer.".to_owned()),
            subscribers: Some("1.2M subscribers".to_owned()),
            is_subscribed: false,
            artwork_url: None,
            top_songs: vec![sample_track("s1", "First Light")],
            albums: vec![LibraryCollectionRef {
                browse_id: "MPREb_1".to_owned(),
                playlist_id: None,
                title: "Dawn Chorus".to_owned(),
                subtitle: "2025".to_owned(),
                artwork_url: None,
                track_count: Some(10),
            }],
            singles: Vec::new(),
            related_artists: Vec::new(),
            radio_playlist_id: None,
            shuffle_playlist_id: None,
        };
        let album = AlbumDetail {
            browse_id: "MPREb_1".to_owned(),
            title: "Dawn Chorus".to_owned(),
            release_type: "ALBUM".to_owned(),
            release_year: Some("2025".to_owned()),
            artists: vec![ArtistRef {
                id: Some("UC_ART_1".to_owned()),
                name: "Kaelen Vance".to_owned(),
            }],
            artwork_url: None,
            total_duration_ms: 210_000,
            tracks: vec![sample_track("s1", "First Light")],
            other_versions: Vec::new(),
            audio_playlist_id: None,
            description: None,
        };

        view()
            .flex_1()
            .child(artist_screen_with_detail(
                artist,
                ArtistScreenCallbacks::default(),
            ))
            .child(album_screen_with_detail(
                album,
                CollectionOfflineProgress::Unpinned,
                AlbumScreenCallbacks::default(),
            ))
    }

    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(Points(1200.0), Points(800.0)),
        environment: Environment {
            color_scheme: ColorScheme::Dark,
            ..Environment::default()
        },
        ..EngineConfig::new(Root::of::<EntityScreensRoot>())
    })
    .expect("headless engine starts cleanly");
    app.wait_for("Kaelen Vance").unwrap();
    app.wait_for("Dawn Chorus").unwrap();
}
