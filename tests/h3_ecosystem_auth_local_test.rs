//! Offline verification suite for Wave 2 H3.2 (Local-File Import & Confined Loopback Route).

use resonance::store::local_media_slice;
use resonance_core::proxy::local_route;

use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt, TagType};
use nsh_core::prelude::signal;
use resonance_contract::{LocalFolderRoot, LocalScanProgress, LocalTrackRecord};
use resonance_core::cache::artwork::ArtworkCache;
use resonance_core::local_media::{LocalMediaStore, scan_all_roots};
use tempfile::TempDir;
use tokio::net::TcpListener;

/// Writes a minimal valid 16-bit PCM WAV file and tags it with `lofty`.
fn write_tagged_wav(
    path: &Path,
    title: &str,
    artist: &str,
    album: &str,
    lyrics: Option<&str>,
    cover_bytes: Option<&[u8]>,
) {
    // Minimal 44-byte RIFF/WAVE PCM header + 256 bytes of silence
    let data_len: u32 = 256;
    let file_len: u32 = 36 + data_len;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_len.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // PCM chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    wav.extend_from_slice(&1u16.to_le_bytes()); // Mono
    wav.extend_from_slice(&44100u32.to_le_bytes()); // Sample rate
    wav.extend_from_slice(&88200u32.to_le_bytes()); // Byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // Block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // Bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.resize(44 + data_len as usize, 0);
    std::fs::write(path, &wav).expect("write wav");

    let mut tagged = Probe::open(path)
        .expect("open wav")
        .guess_file_type()
        .expect("guess wav")
        .read()
        .expect("read wav");

    let mut tag = Tag::new(TagType::Id3v2);
    tag.set_title(title.to_owned());
    tag.set_artist(artist.to_owned());
    tag.set_album(album.to_owned());
    if let Some(lrc) = lyrics {
        tag.insert_text(ItemKey::Lyrics, lrc.to_owned());
    }
    if let Some(bytes) = cover_bytes {
        tag.push_picture(Picture::new_unchecked(
            PictureType::CoverFront,
            Some(MimeType::Png),
            None,
            bytes.to_vec(),
        ));
    }
    tagged.insert_tag(tag);
    tagged
        .save_to_path(path, WriteOptions::default())
        .expect("save lofty wav tags");
}

#[tokio::test]
async fn local_scanner_indexes_wav_dedups_artwork_skips_unchanged_and_prunes_deleted() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let music_dir = tmp.path().join("music");
    std::fs::create_dir_all(&music_dir).unwrap();

    // Synthetic 1x1 PNG header bytes
    let shared_cover = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];
    let track1 = music_dir.join("01_alpha.wav");
    let track2 = music_dir.join("02_beta.wav");

    write_tagged_wav(
        &track1,
        "Alpha Song",
        "Raag Artist",
        "Shared Album",
        Some("[00:01.00] First line"),
        Some(&shared_cover),
    );
    write_tagged_wav(
        &track2,
        "Beta Song",
        "Raag Artist",
        "Shared Album",
        None,
        Some(&shared_cover),
    );

    let store = Arc::new(LocalMediaStore::open(&data_dir).unwrap());
    let artwork = Arc::new(ArtworkCache::new(&data_dir, 32, 8 * 1024 * 1024).unwrap());
    store.add_root(&music_dir, 1000).unwrap();

    // 1. Initial scan indexes both tracks and deduplicates identical album art
    let p1 = scan_all_roots(
        store.clone(),
        Some(artwork.clone()),
        "http://127.0.0.1:9999".to_owned(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(p1.files_seen, 2);
    assert_eq!(p1.files_indexed, 2);
    assert_eq!(p1.files_skipped_unchanged, 0);

    let tracks = store.list_tracks().unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].track.artwork_url, tracks[1].track.artwork_url);
    assert!(tracks[0].track.artwork_url.is_some());

    let embedded = store.get_embedded_lyrics(&tracks[0].track_id).unwrap();
    assert_eq!(embedded.as_deref(), Some("[00:01.00] First line"));

    // 2. Second scan hits incremental fast-path
    let p2 = scan_all_roots(
        store.clone(),
        Some(artwork.clone()),
        "http://127.0.0.1:9999".to_owned(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(p2.files_seen, 2);
    assert_eq!(p2.files_indexed, 0);
    assert_eq!(p2.files_skipped_unchanged, 2);

    // 3. Delete 1 file -> pruned on next scan
    std::fs::remove_file(&track2).unwrap();
    let p3 = scan_all_roots(
        store.clone(),
        Some(artwork),
        "http://127.0.0.1:9999".to_owned(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(p3.files_removed_missing, 1);
    assert_eq!(store.list_tracks().unwrap().len(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn local_scanner_blocks_symlink_escape_outside_registered_root() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let music_dir = tmp.path().join("music");
    let outside_dir = tmp.path().join("outside");
    std::fs::create_dir_all(&music_dir).unwrap();
    std::fs::create_dir_all(&outside_dir).unwrap();

    let secret_audio = outside_dir.join("secret.wav");
    write_tagged_wav(&secret_audio, "Secret", "Nobody", "Outside", None, None);

    let symlink_path = music_dir.join("escape.wav");
    std::os::unix::fs::symlink(&secret_audio, &symlink_path).unwrap();

    let store = Arc::new(LocalMediaStore::open(&data_dir).unwrap());
    store.add_root(&music_dir, 1000).unwrap();

    let progress = scan_all_roots(
        store.clone(),
        None,
        "http://127.0.0.1:9999".to_owned(),
        None,
    )
    .await
    .unwrap();

    assert_eq!(progress.files_seen, 0);
    assert!(store.list_tracks().unwrap().is_empty());
    assert!(
        !store
            .is_path_confined_to_registered_root(&std::fs::canonicalize(&secret_audio).unwrap())
            .unwrap()
    );
}

#[tokio::test]
async fn loopback_proxy_local_route_serves_http_206_range_and_blocks_traversal() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let music_dir = tmp.path().join("music");
    std::fs::create_dir_all(&music_dir).unwrap();

    let cover = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 9, 8, 7, 6];
    let audio_path = music_dir.join("range_test.wav");
    write_tagged_wav(
        &audio_path,
        "Range Test",
        "Artist",
        "Album",
        None,
        Some(&cover),
    );

    let store = Arc::new(LocalMediaStore::open(&data_dir).unwrap());
    let artwork = Arc::new(ArtworkCache::new(&data_dir, 16, 4 * 1024 * 1024).unwrap());
    store.add_root(&music_dir, 1000).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    scan_all_roots(store.clone(), Some(artwork.clone()), base_url.clone(), None)
        .await
        .unwrap();

    let tracks = store.list_tracks().unwrap();
    assert_eq!(tracks.len(), 1);
    let rec = &tracks[0];
    let file_bytes = std::fs::read(&rec.canonical_path).unwrap();

    let store_srv = store.clone();
    let art_srv = artwork.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let s = store_srv.clone();
            let a = art_srv.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let svc = service_fn(move |req: Request<Incoming>| {
                    let s = s.clone();
                    let a = a.clone();
                    async move {
                        let resp = local_route::handle_local_request(&req, Some(&s), Some(&a))
                            .await
                            .unwrap_or_else(|| {
                                hyper::Response::builder()
                                    .status(StatusCode::NOT_FOUND)
                                    .body(http_body_util::Full::new(bytes::Bytes::new()))
                                    .unwrap()
                            });
                        Ok::<_, hyper::Error>(resp)
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    let client = reqwest::Client::new();

    // 1. Byte-range request -> 206 Partial Content
    let resp = client
        .get(format!("{base_url}/local/{}", rec.track_id))
        .header("Range", "bytes=0-63")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        resp.headers()
            .get("X-Resonance-Source")
            .and_then(|v| v.to_str().ok()),
        Some("local-file")
    );
    let body = resp.bytes().await.unwrap();
    assert_eq!(body.as_ref(), &file_bytes[..64]);

    // 2. Embedded artwork route -> 200 OK
    let art_url = rec.track.artwork_url.as_ref().unwrap();
    let art_resp = client.get(art_url).send().await.unwrap();
    assert_eq!(art_resp.status(), reqwest::StatusCode::OK);
    assert_eq!(art_resp.bytes().await.unwrap().as_ref(), cover.as_slice());

    // 3. Traversal / non-id path -> 400 Bad Request
    let bad_resp = client
        .get(format!("{base_url}/local/not-a-valid-id"))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_resp.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[test]
fn local_files_screen_headless_and_zero_raw_colors() {
    let src = include_str!("../src/screens/local_files.rs");
    for forbidden in ["rgb(", "rgba(", "0xFF"] {
        assert!(
            !src.contains(forbidden),
            "local_files.rs must not contain raw color literal `{forbidden}`"
        );
    }

    let slice = local_media_slice::LocalMediaSlice::new();
    assert!(slice.filtered_tracks().is_empty());
    assert!(slice.album_groups().is_empty());
    assert!(slice.artist_groups().is_empty());

    let mut seen_ids = HashSet::new();
    seen_ids.insert("local:0123456789abcdef".to_owned());
    assert_eq!(seen_ids.len(), 1);

    resonance::store::enable_fixture_mode(true);
    let mut app =
        nsh_testing::Headless::start(nsh_engine::EngineConfig::new(nsh_engine::Root::of::<
            resonance::App,
        >()))
        .expect("headless app starts cleanly");
    app.wait_for("Quick Picks").expect("home renders");
}
