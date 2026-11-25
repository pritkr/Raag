//! Offline Store Integration Tests (Option C):
//! 1. SQLite v1 -> v2 migration preserving existing data & surviving `clear_all_caches()`
//! 2. Atomic `.part` download commit + `lofty` M4A metadata, cover art & synced LRC lyrics
//! 3. `ChunkCache` reuse without redundant network fetches
//! 4. Mid-download HTTP 403 URL re-resolution
//! 5. HTTP 429 `Retry-After` / backoff + `.part` preservation on exhaustion + `resume()`
//! 6. Cancel & pause abort cleanly without leaving corrupt final file
//! 7. Startup recovery transitioning interrupted downloads & purging orphan `.part` files
//! 8. `LoopbackProxy` local-disk bypass with zero network calls + `X-Resonance-Source: offline-disk`
//! 9. `LoopbackProxy` generation cancellation (`409`) & self-healing missing offline file
//! 10. `PlaybackController` offline fast-path & 50% next-track pre-resolution skip
//! 11. Security: path traversal, quota enforcement, and secret redaction gates

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{
    ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE, RETRY_AFTER,
};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use lofty::file::TaggedFileExt;
use lofty::tag::{Accessor, ItemKey};
use resonance_contract::{
    AudioBackend, AudioCodec, CacheStore, CoreError, CoreResult, DownloadRecord, DownloadStatus,
    EventBus, HomeSection, LyricLine, LyricSyncKind, LyricsDocument, LyricsProvider, MediaCommand,
    MusicProvider, NowPlayingMetadata, OfflineStorageError, OfflineStore, PlayerState, Playlist,
    SearchResults, StreamInfo, Track,
};
use resonance_core::cache::{ArtworkCache, CHUNK_SIZE, ChunkCache, SqliteCacheStore};
use resonance_core::offline::{DownloadManager, validate_video_id};
use resonance_core::playback::PlaybackController;
use resonance_core::proxy::LoopbackProxy;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify, oneshot};

/// Builds a minimal valid MP4/M4A file (`ftyp` + `moov` + `mdat` >= 520 KB) that `lofty`
/// can probe, read, and write tags to.
fn minimal_valid_m4a_bytes() -> Vec<u8> {
    fn box_wrap(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let len = (8 + payload.len()) as u32;
        let mut out = Vec::with_capacity(len as usize);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(tag);
        out.extend_from_slice(payload);
        out
    }

    fn full_box(tag: &[u8; 4], version: u8, flags: [u8; 3], payload: &[u8]) -> Vec<u8> {
        let mut inner = Vec::with_capacity(4 + payload.len());
        inner.push(version);
        inner.extend_from_slice(&flags);
        inner.extend_from_slice(payload);
        box_wrap(tag, &inner)
    }

    // ftyp: M4A
    let mut ftyp_payload = Vec::new();
    ftyp_payload.extend_from_slice(b"M4A ");
    ftyp_payload.extend_from_slice(&0u32.to_be_bytes());
    ftyp_payload.extend_from_slice(b"M4A ");
    ftyp_payload.extend_from_slice(b"mp42");
    ftyp_payload.extend_from_slice(b"isom");
    let ftyp = box_wrap(b"ftyp", &ftyp_payload);

    // mvhd (version 0: 96 bytes payload after version+flags)
    let mut mvhd_payload = vec![0u8; 96];
    // timescale = 44100 at offset 12..16 (in payload after 4 bytes version+flags -> 8..12)
    mvhd_payload[8..12].copy_from_slice(&44100u32.to_be_bytes());
    // duration = 44100 (1s) at offset 12..16
    mvhd_payload[12..16].copy_from_slice(&44100u32.to_be_bytes());
    // rate = 0x00010000 at 16..20
    mvhd_payload[16..20].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    // volume = 0x0100 at 20..22
    mvhd_payload[20..22].copy_from_slice(&0x0100u16.to_be_bytes());
    // matrix identity at 32..68
    mvhd_payload[32..36].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    mvhd_payload[48..52].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    mvhd_payload[64..68].copy_from_slice(&0x4000_0000u32.to_be_bytes());
    // next_track_ID = 2 at 92..96
    mvhd_payload[92..96].copy_from_slice(&2u32.to_be_bytes());
    let mvhd = full_box(b"mvhd", 0, [0, 0, 0], &mvhd_payload);

    // tkhd (version 0: 80 bytes payload)
    let mut tkhd_payload = vec![0u8; 80];
    tkhd_payload[8..12].copy_from_slice(&1u32.to_be_bytes()); // track_ID = 1
    tkhd_payload[16..20].copy_from_slice(&44100u32.to_be_bytes()); // duration
    tkhd_payload[32..34].copy_from_slice(&0x0100u16.to_be_bytes()); // volume
    tkhd_payload[36..40].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    tkhd_payload[52..56].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    tkhd_payload[68..72].copy_from_slice(&0x4000_0000u32.to_be_bytes());
    let tkhd = full_box(b"tkhd", 0, [0, 0, 7], &tkhd_payload);

    // mdhd (version 0: 20 bytes payload)
    let mut mdhd_payload = vec![0u8; 20];
    mdhd_payload[8..12].copy_from_slice(&44100u32.to_be_bytes());
    mdhd_payload[12..16].copy_from_slice(&44100u32.to_be_bytes());
    let mdhd = full_box(b"mdhd", 0, [0, 0, 0], &mdhd_payload);

    // hdlr
    let mut hdlr_payload = vec![0u8; 21];
    hdlr_payload[4..8].copy_from_slice(b"soun");
    let hdlr = full_box(b"hdlr", 0, [0, 0, 0], &hdlr_payload);

    // smhd
    let smhd = full_box(b"smhd", 0, [0, 0, 0], &[0u8; 4]);

    // dinf -> dref -> url
    let url_box = full_box(b"url ", 0, [0, 0, 1], &[]);
    let mut dref_payload = Vec::new();
    dref_payload.extend_from_slice(&1u32.to_be_bytes());
    dref_payload.extend_from_slice(&url_box);
    let dref = full_box(b"dref", 0, [0, 0, 0], &dref_payload);
    let dinf = box_wrap(b"dinf", &dref);

    // stsd -> mp4a -> esds (valid ES_DescrTag -> DecoderConfigDescrTag -> DecoderSpecificInfoTag -> SLConfigDescrTag)
    let esds_payload: [u8; 34] = [
        0x03, 0x19, 0x00, 0x01, 0x00, // ES_DescrTag (len 25, ES_ID=1, flags=0)
        0x04, 0x11, 0x40, 0x15, 0x00, 0x00, 0x00, // DecoderConfigDescrTag (AAC, AudioStream)
        0x00, 0x01, 0xF4, 0x00, // maxBitrate = 128000
        0x00, 0x01, 0xF4, 0x00, // avgBitrate = 128000
        0x05, 0x02, 0x12, 0x10, // DecoderSpecificInfoTag (AAC-LC 44100Hz stereo)
        0x06, 0x01, 0x02, // SLConfigDescrTag
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    let esds = full_box(b"esds", 0, [0, 0, 0], &esds_payload[..27]);
    let mut mp4a_payload = vec![0u8; 28];
    mp4a_payload[6..8].copy_from_slice(&1u16.to_be_bytes()); // data_reference_index
    mp4a_payload[16..18].copy_from_slice(&2u16.to_be_bytes()); // channelcount
    mp4a_payload[18..20].copy_from_slice(&16u16.to_be_bytes()); // samplesize
    mp4a_payload[24..28].copy_from_slice(&(44100u32 << 16).to_be_bytes()); // samplerate
    mp4a_payload.extend_from_slice(&esds);
    let mp4a = box_wrap(b"mp4a", &mp4a_payload);
    let mut stsd_payload = Vec::new();
    stsd_payload.extend_from_slice(&1u32.to_be_bytes());
    stsd_payload.extend_from_slice(&mp4a);
    let stsd = full_box(b"stsd", 0, [0, 0, 0], &stsd_payload);

    let mut stts_payload = Vec::new();
    stts_payload.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1
    stts_payload.extend_from_slice(&1u32.to_be_bytes()); // sample_count = 1
    stts_payload.extend_from_slice(&1024u32.to_be_bytes()); // sample_delta = 1024
    let stts = full_box(b"stts", 0, [0, 0, 0], &stts_payload);

    let mut stsc_payload = Vec::new();
    stsc_payload.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1
    stsc_payload.extend_from_slice(&1u32.to_be_bytes()); // first_chunk = 1
    stsc_payload.extend_from_slice(&1u32.to_be_bytes()); // samples_per_chunk = 1
    stsc_payload.extend_from_slice(&1u32.to_be_bytes()); // sample_description_index = 1
    let stsc = full_box(b"stsc", 0, [0, 0, 0], &stsc_payload);

    let mut stsz_payload = Vec::new();
    stsz_payload.extend_from_slice(&0u32.to_be_bytes()); // sample_size = 0 (variable)
    stsz_payload.extend_from_slice(&1u32.to_be_bytes()); // sample_count = 1
    stsz_payload.extend_from_slice(&1024u32.to_be_bytes()); // entry_size[0] = 1024
    let stsz = full_box(b"stsz", 0, [0, 0, 0], &stsz_payload);

    let stco_placeholder_offset = 0u32;
    let mut stco_payload = Vec::new();
    stco_payload.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1
    stco_payload.extend_from_slice(&stco_placeholder_offset.to_be_bytes());
    let stco = full_box(b"stco", 0, [0, 0, 0], &stco_payload);

    let mut stbl_inner = Vec::new();
    stbl_inner.extend_from_slice(&stsd);
    stbl_inner.extend_from_slice(&stts);
    stbl_inner.extend_from_slice(&stsc);
    stbl_inner.extend_from_slice(&stsz);
    stbl_inner.extend_from_slice(&stco);
    let stbl = box_wrap(b"stbl", &stbl_inner);

    let mut minf_inner = Vec::new();
    minf_inner.extend_from_slice(&smhd);
    minf_inner.extend_from_slice(&dinf);
    minf_inner.extend_from_slice(&stbl);
    let minf = box_wrap(b"minf", &minf_inner);

    let mut mdia_inner = Vec::new();
    mdia_inner.extend_from_slice(&mdhd);
    mdia_inner.extend_from_slice(&hdlr);
    mdia_inner.extend_from_slice(&minf);
    let mdia = box_wrap(b"mdia", &mdia_inner);

    let mut trak_inner = Vec::new();
    trak_inner.extend_from_slice(&tkhd);
    trak_inner.extend_from_slice(&mdia);
    let trak = box_wrap(b"trak", &trak_inner);

    let mut moov_inner = Vec::new();
    moov_inner.extend_from_slice(&mvhd);
    moov_inner.extend_from_slice(&trak);
    let mut moov = box_wrap(b"moov", &moov_inner);

    // Patch stco chunk_offset to point right after ftyp + moov + mdat header (8 bytes)
    let mdat_data_offset = (ftyp.len() + moov.len() + 8) as u32;
    let stco_offset_pos = moov.len() - 4;
    moov[stco_offset_pos..].copy_from_slice(&mdat_data_offset.to_be_bytes());

    // mdat >= 520 KB so it spans > 2 chunks of 256 KB
    let mdat_payload = vec![0xA5u8; (CHUNK_SIZE as usize) * 2 + 16_384];
    let mdat = box_wrap(b"mdat", &mdat_payload);

    let mut out = Vec::with_capacity(ftyp.len() + moov.len() + mdat.len());
    out.extend_from_slice(&ftyp);
    out.extend_from_slice(&moov);
    out.extend_from_slice(&mdat);
    out
}

/// 1x1 transparent PNG bytes.
fn minimal_valid_png_bytes() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15,
        0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, // IDAT
        0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
        0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82, // IEND
    ]
}

fn sample_track(id: &str) -> Track {
    Track {
        id: id.to_owned(),
        title: format!("Track {id}"),
        artists: vec![resonance_contract::ArtistRef {
            id: Some("art_1".into()),
            name: "Test Artist".into(),
        }],
        album: Some(resonance_contract::AlbumRef {
            id: Some("alb_1".into()),
            title: "Test Album".into(),
        }),
        duration_ms: 180_000,
        artwork_url: None,
        is_explicit: false,
        is_liked: false,
        is_video: false,
    }
}

fn parse_range(hdr: &str, total_len: usize) -> Option<(usize, usize)> {
    let rest = hdr.strip_prefix("bytes=")?;
    let (s, e) = rest.split_once('-')?;
    let start: usize = s.parse().ok()?;
    let end: usize = if e.is_empty() {
        total_len.saturating_sub(1)
    } else {
        e.parse::<usize>().ok()?.min(total_len.saturating_sub(1))
    };
    if start <= end && start < total_len {
        Some((start, end))
    } else {
        None
    }
}

#[derive(Clone)]
struct CdnScript {
    payload: Arc<Vec<u8>>,
    artwork: Arc<Vec<u8>>,
    chunk_scripts: Arc<Mutex<HashMap<(u64, usize), StatusCode>>>,
    attempt_counts: Arc<Mutex<HashMap<u64, usize>>>,
    requested_chunks: Arc<Mutex<Vec<u64>>>,
    total_requests: Arc<AtomicUsize>,
    block_chunk_1: Arc<AtomicBool>,
    chunk_1_entered: Arc<Notify>,
    chunk_1_release: Arc<Notify>,
}

struct MockUpstreamCdn {
    base_url: String,
    script: CdnScript,
    shutdown: Option<oneshot::Sender<()>>,
}

impl MockUpstreamCdn {
    async fn start(payload: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, mut rx) = oneshot::channel();
        let script = CdnScript {
            payload: Arc::new(payload),
            artwork: Arc::new(minimal_valid_png_bytes()),
            chunk_scripts: Arc::new(Mutex::new(HashMap::new())),
            attempt_counts: Arc::new(Mutex::new(HashMap::new())),
            requested_chunks: Arc::new(Mutex::new(Vec::new())),
            total_requests: Arc::new(AtomicUsize::new(0)),
            block_chunk_1: Arc::new(AtomicBool::new(false)),
            chunk_1_entered: Arc::new(Notify::new()),
            chunk_1_release: Arc::new(Notify::new()),
        };

        let s_clone = script.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    accept = listener.accept() => {
                        let Ok((stream, _)) = accept else { break };
                        let sc = s_clone.clone();
                        tokio::spawn(async move {
                            let io = TokioIo::new(stream);
                            let svc = service_fn(move |req: Request<Incoming>| {
                                let sc = sc.clone();
                                async move {
                                    let path = req.uri().path().to_owned();
                                    if path == "/cover.png" {
                                        return Ok::<_, hyper::Error>(
                                            Response::builder()
                                                .status(StatusCode::OK)
                                                .header(CONTENT_TYPE, "image/png")
                                                .body(Full::new(Bytes::from((*sc.artwork).clone())))
                                                .unwrap(),
                                        );
                                    }

                                    sc.total_requests.fetch_add(1, Ordering::SeqCst);
                                    let total = sc.payload.len();
                                    let range_hdr = req
                                        .headers()
                                        .get(RANGE)
                                        .and_then(|v| v.to_str().ok())
                                        .map(|s| s.to_owned());

                                    let (start, end, is_range) = if let Some(ref h) = range_hdr {
                                        let Some((s, e)) = parse_range(h, total) else {
                                            return Ok(Response::builder()
                                                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                                                .body(Full::new(Bytes::new()))
                                                .unwrap());
                                        };
                                        (s, e, true)
                                    } else {
                                        (0, total.saturating_sub(1), false)
                                    };

                                    let chunk_idx = (start as u64) / CHUNK_SIZE;
                                    sc.requested_chunks.lock().await.push(chunk_idx);

                                    if chunk_idx == 1 && sc.block_chunk_1.load(Ordering::SeqCst) {
                                        sc.chunk_1_entered.notify_waiters();
                                        sc.chunk_1_release.notified().await;
                                    }

                                    let attempt = {
                                        let mut counts = sc.attempt_counts.lock().await;
                                        let entry = counts.entry(chunk_idx).or_insert(0);
                                        let cur = *entry;
                                        *entry += 1;
                                        cur
                                    };

                                    if let Some(status) = sc
                                        .chunk_scripts
                                        .lock()
                                        .await
                                        .get(&(chunk_idx, attempt))
                                        .copied()
                                    {
                                        let mut b = Response::builder().status(status);
                                        if status == StatusCode::TOO_MANY_REQUESTS {
                                            b = b.header(RETRY_AFTER, "0");
                                        }
                                        return Ok(b.body(Full::new(Bytes::new())).unwrap());
                                    }

                                    let slice = Bytes::copy_from_slice(&sc.payload[start..=end]);
                                    if is_range {
                                        Ok(Response::builder()
                                            .status(StatusCode::PARTIAL_CONTENT)
                                            .header(CONTENT_TYPE, "audio/mp4")
                                            .header(ACCEPT_RANGES, "bytes")
                                            .header(
                                                CONTENT_RANGE,
                                                format!("bytes {start}-{end}/{total}"),
                                            )
                                            .header(CONTENT_LENGTH, slice.len().to_string())
                                            .body(Full::new(slice))
                                            .unwrap())
                                    } else {
                                        Ok(Response::builder()
                                            .status(StatusCode::OK)
                                            .header(CONTENT_TYPE, "audio/mp4")
                                            .header(ACCEPT_RANGES, "bytes")
                                            .header(CONTENT_LENGTH, slice.len().to_string())
                                            .body(Full::new(slice))
                                            .unwrap())
                                    }
                                }
                            });
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(io, svc)
                                .await;
                        });
                    }
                }
            }
        });

        Self {
            base_url: format!("http://{addr}"),
            script,
            shutdown: Some(tx),
        }
    }

    #[allow(dead_code)]
    fn media_url(&self, version: usize) -> String {
        format!(
            "{}/videoplayback?v={version}&sig=SECRET_SIG&n=SECRET_N&pot=SECRET_POT&ip=1.2.3.4",
            self.base_url
        )
    }

    fn cover_url(&self) -> String {
        format!("{}/cover.png", self.base_url)
    }
}

impl Drop for MockUpstreamCdn {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

struct MockDownloadProvider {
    base_url: String,
    payload_len: u64,
    resolve_calls: AtomicUsize,
    force_fail: AtomicBool,
}

impl MockDownloadProvider {
    fn new(base_url: String, payload_len: u64) -> Self {
        Self {
            base_url,
            payload_len,
            resolve_calls: AtomicUsize::new(0),
            force_fail: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl MusicProvider for MockDownloadProvider {
    async fn search(&self, query: &str) -> CoreResult<SearchResults> {
        Ok(SearchResults {
            query: query.to_owned(),
            tracks: vec![],
            albums: vec![],
            continuation: None,
        })
    }
    async fn home(&self) -> CoreResult<Vec<HomeSection>> {
        Ok(vec![])
    }
    async fn browse_collection(&self, browse_id: &str) -> CoreResult<Playlist> {
        Ok(Playlist {
            id: browse_id.to_owned(),
            title: "Mock".into(),
            subtitle: None,
            artwork_url: None,
            tracks: vec![],
        })
    }
    async fn liked_songs(&self) -> CoreResult<Vec<Track>> {
        Ok(vec![])
    }
    async fn autoplay_next(&self, _video_id: &str) -> CoreResult<Vec<Track>> {
        Ok(vec![])
    }
    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo> {
        let ver = self.resolve_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.force_fail.load(Ordering::SeqCst) {
            return Err(CoreError::Offline(format!(
                "upstream failed for {}/videoplayback?sig=SECRET_SIG&n=SECRET_N&pot=SECRET_POT&SAPISID=SECRET_COOKIE",
                self.base_url
            )));
        }
        Ok(StreamInfo {
            video_id: video_id.to_owned(),
            url: format!(
                "{}/videoplayback?v={ver}&sig=SECRET_SIG&n=SECRET_N&pot=SECRET_POT&ip=1.2.3.4",
                self.base_url
            ),
            itag: 140,
            mime_type: "audio/mp4".into(),
            codec: AudioCodec::Aac,
            bitrate_kbps: 128,
            content_length: self.payload_len,
            duration_ms: 180_000,
            loudness_db: None,
            resolved_by_client: "WEB_REMIX".into(),
            expires_at_unix_secs: 4_000_000_000,
        })
    }
    async fn ytm_lyrics(&self, _video_id: &str) -> CoreResult<Option<LyricsDocument>> {
        Ok(None)
    }
}

struct MockLyrics {
    doc: Option<LyricsDocument>,
}

#[async_trait]
impl LyricsProvider for MockLyrics {
    async fn fetch_lyrics(&self, _track: &Track) -> CoreResult<Option<LyricsDocument>> {
        Ok(self.doc.clone())
    }
}

struct MockAudioBackend {
    loaded_urls: Mutex<Vec<String>>,
}

impl MockAudioBackend {
    fn new() -> Self {
        Self {
            loaded_urls: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl AudioBackend for MockAudioBackend {
    async fn load_and_play(&self, proxy_url: &str, _meta: NowPlayingMetadata) -> CoreResult<()> {
        self.loaded_urls.lock().await.push(proxy_url.to_owned());
        Ok(())
    }
    async fn play(&self) -> CoreResult<()> {
        Ok(())
    }
    async fn pause(&self) -> CoreResult<()> {
        Ok(())
    }
    async fn stop(&self) -> CoreResult<()> {
        Ok(())
    }
    async fn seek_ms(&self, _position_ms: u64) -> CoreResult<()> {
        Ok(())
    }
    async fn set_volume(&self, _volume: f32) -> CoreResult<()> {
        Ok(())
    }
    async fn position_ms(&self) -> u64 {
        0
    }
    async fn update_now_playing(&self, _meta: NowPlayingMetadata, _playing: bool, _pos: u64) {}
    fn take_command_receiver(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<MediaCommand>> {
        None
    }
}

async fn wait_for_status(
    store: &SqliteCacheStore,
    video_id: &str,
    predicate: impl Fn(DownloadStatus) -> bool,
) -> DownloadRecord {
    for _ in 0..200 {
        if let Ok(Some(rec)) = store.get_download(video_id)
            && predicate(rec.status)
        {
            return rec;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for download status on {video_id}");
}

#[test]
fn sqlite_v1_to_v2_migration_preserves_existing_data_and_creates_downloads_table() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("resonance.db");
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE kv_store (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE metadata_cache (cache_key TEXT PRIMARY KEY, payload_json TEXT NOT NULL, updated_at INTEGER NOT NULL, ttl_secs INTEGER NOT NULL);
            CREATE TABLE listening_history (id INTEGER PRIMARY KEY AUTOINCREMENT, track_id TEXT NOT NULL, track_json TEXT NOT NULL, played_at INTEGER NOT NULL);
            CREATE TABLE audio_chunks (video_id TEXT NOT NULL, itag INTEGER NOT NULL, chunk_index INTEGER NOT NULL, byte_len INTEGER NOT NULL, total_len INTEGER NOT NULL, sha256_hex TEXT NOT NULL, last_accessed INTEGER NOT NULL, PRIMARY KEY (video_id, itag, chunk_index));
            INSERT INTO kv_store (key, value) VALUES ('visitor_data', 'VISITOR_V1');
            INSERT INTO listening_history (track_id, track_json, played_at) VALUES ('vid1', '{\"id\":\"vid1\",\"title\":\"Old\",\"artists\":[],\"album\":null,\"duration_ms\":1000,\"artwork_url\":null,\"is_explicit\":false,\"is_liked\":false,\"is_video\":false}', 12345);
            PRAGMA user_version = 1;
            ",
        )
        .unwrap();
    }

    let store = SqliteCacheStore::open(dir.path()).unwrap();
    assert_eq!(
        store.get_visitor_data().unwrap().as_deref(),
        Some("VISITOR_V1")
    );
    assert_eq!(store.load_history(10).unwrap().len(), 1);

    let conn = Connection::open(&db_path).unwrap();
    let version: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert!(
        version >= 2,
        "expected schema migration to at least v2, got v{version}"
    );

    let rec = DownloadRecord {
        video_id: "offline_v1".into(),
        track: sample_track("offline_v1"),
        status: DownloadStatus::Completed,
        itag: 140,
        codec: AudioCodec::Aac,
        mime_type: "audio/mp4".into(),
        relative_path: Some("offline_v1_140.m4a".into()),
        bytes_downloaded: 1024,
        total_bytes: 1024,
        sha256_hex: Some("abc".into()),
        tags_embedded: true,
        created_at_unix_secs: 100,
        updated_at_unix_secs: 100,
        completed_at_unix_secs: Some(100),
        error_message: None,
    };
    store.upsert_download(&rec).unwrap();
    store.clear_all_caches().unwrap();
    assert!(
        store
            .get_completed_download("offline_v1")
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn atomic_part_download_commits_final_file_and_embeds_lofty_m4a_tags() {
    let payload = minimal_valid_m4a_bytes();
    let cdn = MockUpstreamCdn::start(payload.clone()).await;
    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let chunks = Arc::new(ChunkCache::new(dir.path(), cache.clone(), 64 * 1024 * 1024).unwrap());
    let artwork = Arc::new(ArtworkCache::new(dir.path(), 16, 16 * 1024 * 1024).unwrap());
    let provider = Arc::new(MockDownloadProvider::new(
        cdn.base_url.clone(),
        payload.len() as u64,
    ));
    let lyrics = Arc::new(MockLyrics {
        doc: Some(LyricsDocument {
            video_id: "m4a_tag_1".into(),
            provider: "Mock".into(),
            sync_kind: LyricSyncKind::Line,
            lines: vec![
                LyricLine {
                    start_ms: 1_250,
                    end_ms: 4_000,
                    text: "First synced line".into(),
                    words: vec![],
                    is_instrumental: false,
                },
                LyricLine {
                    start_ms: 65_500,
                    end_ms: 70_000,
                    text: "Second synced line".into(),
                    words: vec![],
                    is_instrumental: false,
                },
            ],
        }),
    });
    let bus = EventBus::new(64);
    let mgr = DownloadManager::new(provider, cache.clone(), chunks, artwork, lyrics, bus).unwrap();

    let mut track = sample_track("m4a_tag_1");
    track.artwork_url = Some(cdn.cover_url());

    mgr.enqueue(track.clone()).await.unwrap();
    let rec = wait_for_status(&cache, "m4a_tag_1", |s| s == DownloadStatus::Completed).await;
    assert!(rec.tags_embedded);

    let root = cache.downloads_root();
    let final_path = root.join(rec.relative_path.as_deref().unwrap());
    let part_path = root.join(format!("{}.part", rec.relative_path.as_deref().unwrap()));
    assert!(final_path.is_file());
    assert!(!part_path.exists());

    let disk_bytes = std::fs::read(&final_path).unwrap();
    let digest = hex::encode(Sha256::digest(&disk_bytes));
    assert_eq!(rec.sha256_hex.as_deref(), Some(digest.as_str()));

    let tagged = lofty::read_from_path(&final_path).unwrap();
    let tag = tagged.primary_tag().unwrap();
    assert_eq!(tag.title().as_deref(), Some(track.title.as_str()));
    assert_eq!(tag.artist().as_deref(), Some("Test Artist"));
    assert_eq!(tag.album().as_deref(), Some("Test Album"));
    let cover = tag.pictures().first().expect("cover picture missing");
    assert_eq!(cover.data(), minimal_valid_png_bytes().as_slice());
    let lrc = tag.get_string(&ItemKey::Lyrics).unwrap();
    assert!(lrc.contains("[00:01.25]First synced line"));
    assert!(lrc.contains("[01:05.50]Second synced line"));
}

#[tokio::test]
async fn download_worker_reuses_chunk_cache_without_redundant_network_fetches() {
    let payload = minimal_valid_m4a_bytes();
    let cdn = MockUpstreamCdn::start(payload.clone()).await;
    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let chunks = Arc::new(ChunkCache::new(dir.path(), cache.clone(), 64 * 1024 * 1024).unwrap());
    let artwork = Arc::new(ArtworkCache::new(dir.path(), 16, 16 * 1024 * 1024).unwrap());

    // Pre-populate chunk 0 in ChunkCache
    let c0_end = CHUNK_SIZE as usize;
    chunks
        .write_chunk(
            "chunk_reuse_1",
            140,
            0,
            &payload[..c0_end],
            payload.len() as u64,
        )
        .unwrap();

    let provider = Arc::new(MockDownloadProvider::new(
        cdn.base_url.clone(),
        payload.len() as u64,
    ));
    let mgr = DownloadManager::new(
        provider,
        cache.clone(),
        chunks,
        artwork,
        Arc::new(MockLyrics { doc: None }),
        EventBus::new(64),
    )
    .unwrap();

    mgr.enqueue(sample_track("chunk_reuse_1")).await.unwrap();
    wait_for_status(&cache, "chunk_reuse_1", |s| s == DownloadStatus::Completed).await;

    let reqs = cdn.script.requested_chunks.lock().await.clone();
    assert!(
        !reqs.contains(&0),
        "chunk 0 should come from ChunkCache, got {reqs:?}"
    );
    assert!(reqs.contains(&1));
}

#[tokio::test]
async fn download_worker_recovers_mid_download_http_403_via_url_reresolution() {
    let payload = minimal_valid_m4a_bytes();
    let cdn = MockUpstreamCdn::start(payload.clone()).await;
    cdn.script
        .chunk_scripts
        .lock()
        .await
        .insert((1, 0), StatusCode::FORBIDDEN);

    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let chunks = Arc::new(ChunkCache::new(dir.path(), cache.clone(), 64 * 1024 * 1024).unwrap());
    let artwork = Arc::new(ArtworkCache::new(dir.path(), 16, 16 * 1024 * 1024).unwrap());
    let provider = Arc::new(MockDownloadProvider::new(
        cdn.base_url.clone(),
        payload.len() as u64,
    ));
    let mgr = DownloadManager::new(
        provider.clone(),
        cache.clone(),
        chunks,
        artwork,
        Arc::new(MockLyrics { doc: None }),
        EventBus::new(64),
    )
    .unwrap();

    mgr.enqueue(sample_track("http403_1")).await.unwrap();
    let rec = wait_for_status(&cache, "http403_1", |s| s == DownloadStatus::Completed).await;
    assert_eq!(provider.resolve_calls.load(Ordering::SeqCst), 2);
    let final_path = cache.downloads_root().join(rec.relative_path.unwrap());
    let expected_sha = hex::encode(Sha256::digest(std::fs::read(final_path).unwrap()));
    assert_eq!(rec.sha256_hex.as_deref(), Some(expected_sha.as_str()));
}

#[tokio::test]
async fn download_worker_handles_http_429_backoff_and_preserves_part_on_exhaustion() {
    let payload = minimal_valid_m4a_bytes();
    let cdn = MockUpstreamCdn::start(payload.clone()).await;
    {
        let mut scripts = cdn.script.chunk_scripts.lock().await;
        scripts.insert((0, 0), StatusCode::TOO_MANY_REQUESTS);
        scripts.insert((0, 1), StatusCode::TOO_MANY_REQUESTS);
    }

    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let chunks = Arc::new(ChunkCache::new(dir.path(), cache.clone(), 64 * 1024 * 1024).unwrap());
    let artwork = Arc::new(ArtworkCache::new(dir.path(), 16, 16 * 1024 * 1024).unwrap());
    let provider = Arc::new(MockDownloadProvider::new(
        cdn.base_url.clone(),
        payload.len() as u64,
    ));
    let mgr = DownloadManager::new(
        provider,
        cache.clone(),
        chunks,
        artwork,
        Arc::new(MockLyrics { doc: None }),
        EventBus::new(64),
    )
    .unwrap();

    // First track: 2 x 429 then 206 -> Completed
    mgr.enqueue(sample_track("http429_ok")).await.unwrap();
    wait_for_status(&cache, "http429_ok", |s| s == DownloadStatus::Completed).await;

    // Second track: persistent 429 on chunk 1 -> Paused with .part preserved, then resume() completes
    let cdn2 = MockUpstreamCdn::start(payload.clone()).await;
    {
        let mut scripts = cdn2.script.chunk_scripts.lock().await;
        for attempt in 0..5 {
            scripts.insert((1, attempt), StatusCode::TOO_MANY_REQUESTS);
        }
    }
    let provider2 = Arc::new(MockDownloadProvider::new(
        cdn2.base_url.clone(),
        payload.len() as u64,
    ));
    let dir2 = tempdir().unwrap();
    let cache2 = Arc::new(SqliteCacheStore::open(dir2.path()).unwrap());
    let chunks2 = Arc::new(ChunkCache::new(dir2.path(), cache2.clone(), 64 * 1024 * 1024).unwrap());
    let artwork2 = Arc::new(ArtworkCache::new(dir2.path(), 16, 16 * 1024 * 1024).unwrap());
    let mgr2 = DownloadManager::new(
        provider2,
        cache2.clone(),
        chunks2,
        artwork2,
        Arc::new(MockLyrics { doc: None }),
        EventBus::new(64),
    )
    .unwrap();

    mgr2.enqueue(sample_track("http429_pause")).await.unwrap();
    wait_for_status(&cache2, "http429_pause", |s| s == DownloadStatus::Paused).await;
    let part_path = cache2.downloads_root().join("http429_pause_140.m4a.part");
    assert!(part_path.is_file());
    assert!(std::fs::metadata(&part_path).unwrap().len() >= CHUNK_SIZE);

    cdn2.script.chunk_scripts.lock().await.clear();
    mgr2.resume("http429_pause").await.unwrap();
    wait_for_status(&cache2, "http429_pause", |s| s == DownloadStatus::Completed).await;
}

#[tokio::test]
async fn download_cancel_and_pause_abort_cleanly_without_leaving_corrupt_final_file() {
    let payload = minimal_valid_m4a_bytes();
    let cdn = MockUpstreamCdn::start(payload.clone()).await;
    cdn.script.block_chunk_1.store(true, Ordering::SeqCst);

    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let chunks = Arc::new(ChunkCache::new(dir.path(), cache.clone(), 64 * 1024 * 1024).unwrap());
    let artwork = Arc::new(ArtworkCache::new(dir.path(), 16, 16 * 1024 * 1024).unwrap());
    let provider = Arc::new(MockDownloadProvider::new(
        cdn.base_url.clone(),
        payload.len() as u64,
    ));
    let mgr = DownloadManager::new(
        provider,
        cache.clone(),
        chunks,
        artwork,
        Arc::new(MockLyrics { doc: None }),
        EventBus::new(64),
    )
    .unwrap();

    mgr.enqueue(sample_track("cancel_1")).await.unwrap();
    cdn.script.chunk_1_entered.notified().await;
    mgr.cancel("cancel_1").await.unwrap();
    cdn.script.chunk_1_release.notify_waiters();

    let rec = wait_for_status(&cache, "cancel_1", |s| s == DownloadStatus::Cancelled).await;
    assert_eq!(rec.status, DownloadStatus::Cancelled);
    assert!(
        !cache
            .downloads_root()
            .join("cancel_1_140.m4a.part")
            .exists()
    );
    assert!(!cache.downloads_root().join("cancel_1_140.m4a").exists());

    // Pause then resume
    cdn.script.block_chunk_1.store(true, Ordering::SeqCst);
    mgr.enqueue(sample_track("pause_1")).await.unwrap();
    cdn.script.chunk_1_entered.notified().await;
    mgr.pause("pause_1").await.unwrap();
    cdn.script.chunk_1_release.notify_waiters();

    wait_for_status(&cache, "pause_1", |s| s == DownloadStatus::Paused).await;
    assert!(
        cache
            .downloads_root()
            .join("pause_1_140.m4a.part")
            .is_file()
    );
    assert!(!cache.downloads_root().join("pause_1_140.m4a").exists());

    cdn.script.block_chunk_1.store(false, Ordering::SeqCst);
    mgr.resume("pause_1").await.unwrap();
    wait_for_status(&cache, "pause_1", |s| s == DownloadStatus::Completed).await;
}

#[tokio::test]
async fn startup_recovery_transitions_interrupted_downloads_and_purges_orphan_parts() {
    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let chunks = Arc::new(ChunkCache::new(dir.path(), cache.clone(), 64 * 1024 * 1024).unwrap());
    let artwork = Arc::new(ArtworkCache::new(dir.path(), 16, 16 * 1024 * 1024).unwrap());
    let provider = Arc::new(MockDownloadProvider::new("http://127.0.0.1:0".into(), 1024));

    let part_path = cache.downloads_root().join("interrupted_1_140.m4a.part");
    std::fs::write(&part_path, vec![1u8; 512]).unwrap();
    let orphan_path = cache.downloads_root().join("orphan_140.m4a.part");
    std::fs::write(&orphan_path, vec![2u8; 128]).unwrap();

    cache
        .upsert_download(&DownloadRecord {
            video_id: "interrupted_1".into(),
            track: sample_track("interrupted_1"),
            status: DownloadStatus::Downloading,
            itag: 140,
            codec: AudioCodec::Aac,
            mime_type: "audio/mp4".into(),
            relative_path: None,
            bytes_downloaded: 512,
            total_bytes: 1024,
            sha256_hex: None,
            tags_embedded: false,
            created_at_unix_secs: 10,
            updated_at_unix_secs: 10,
            completed_at_unix_secs: None,
            error_message: None,
        })
        .unwrap();

    let mgr = DownloadManager::new(
        provider,
        cache.clone(),
        chunks,
        artwork,
        Arc::new(MockLyrics { doc: None }),
        EventBus::new(64),
    )
    .unwrap();
    let recovered = mgr.startup_recovery().await.unwrap();
    assert_eq!(recovered, 1);

    let rec = cache.get_download("interrupted_1").unwrap().unwrap();
    assert_eq!(rec.status, DownloadStatus::Paused);
    assert!(!orphan_path.exists());
}

#[tokio::test]
async fn loopback_proxy_serves_completed_download_from_local_disk_with_zero_network_calls() {
    let payload = minimal_valid_m4a_bytes();
    let cdn = MockUpstreamCdn::start(payload.clone()).await;
    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let chunks = Arc::new(ChunkCache::new(dir.path(), cache.clone(), 64 * 1024 * 1024).unwrap());
    let artwork = Arc::new(ArtworkCache::new(dir.path(), 16, 16 * 1024 * 1024).unwrap());
    let provider = Arc::new(MockDownloadProvider::new(
        cdn.base_url.clone(),
        payload.len() as u64,
    ));
    let mgr = DownloadManager::new(
        provider.clone(),
        cache.clone(),
        chunks.clone(),
        artwork,
        Arc::new(MockLyrics { doc: None }),
        EventBus::new(64),
    )
    .unwrap();

    mgr.enqueue(sample_track("proxy_disk_1")).await.unwrap();
    let rec = wait_for_status(&cache, "proxy_disk_1", |s| s == DownloadStatus::Completed).await;
    let disk_bytes =
        std::fs::read(cache.downloads_root().join(rec.relative_path.unwrap())).unwrap();

    provider.resolve_calls.store(0, Ordering::SeqCst);
    cdn.script.total_requests.store(0, Ordering::SeqCst);

    let proxy = LoopbackProxy::start_with_offline(provider.clone(), chunks, Some(cache.clone()))
        .await
        .unwrap();
    let url = proxy.stream_url_with_gen("proxy_disk_1", 140, proxy.current_generation());

    let client = reqwest::Client::new();
    let full_resp = client.get(&url).send().await.unwrap();
    assert_eq!(full_resp.status(), StatusCode::OK);
    assert_eq!(
        full_resp
            .headers()
            .get("X-Resonance-Source")
            .and_then(|v| v.to_str().ok()),
        Some("offline-disk")
    );
    assert_eq!(
        full_resp.bytes().await.unwrap().as_ref(),
        disk_bytes.as_slice()
    );

    let range_resp = client
        .get(&url)
        .header(RANGE, "bytes=1024-4095")
        .send()
        .await
        .unwrap();
    assert_eq!(range_resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        range_resp
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok()),
        Some(format!("bytes 1024-4095/{}", disk_bytes.len()).as_str())
    );
    assert_eq!(
        range_resp.bytes().await.unwrap().as_ref(),
        &disk_bytes[1024..=4095]
    );
    assert_eq!(provider.resolve_calls.load(Ordering::SeqCst), 0);
    assert_eq!(cdn.script.total_requests.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn loopback_proxy_enforces_generation_cancellation_and_self_heals_missing_offline_file() {
    let payload = minimal_valid_m4a_bytes();
    let cdn = MockUpstreamCdn::start(payload.clone()).await;
    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let chunks = Arc::new(ChunkCache::new(dir.path(), cache.clone(), 64 * 1024 * 1024).unwrap());
    let artwork = Arc::new(ArtworkCache::new(dir.path(), 16, 16 * 1024 * 1024).unwrap());
    let provider = Arc::new(MockDownloadProvider::new(
        cdn.base_url.clone(),
        payload.len() as u64,
    ));
    let mgr = DownloadManager::new(
        provider.clone(),
        cache.clone(),
        chunks.clone(),
        artwork,
        Arc::new(MockLyrics { doc: None }),
        EventBus::new(64),
    )
    .unwrap();

    mgr.enqueue(sample_track("heal_1")).await.unwrap();
    let rec = wait_for_status(&cache, "heal_1", |s| s == DownloadStatus::Completed).await;
    let final_path = cache.downloads_root().join(rec.relative_path.unwrap());

    let proxy = LoopbackProxy::start_with_offline(provider.clone(), chunks, Some(cache.clone()))
        .await
        .unwrap();
    let stale_url = proxy.stream_url_with_gen("heal_1", 140, 0);
    let _new_gen = proxy.advance_generation();

    let client = reqwest::Client::new();
    let stale_resp = client.get(&stale_url).send().await.unwrap();
    assert_eq!(stale_resp.status(), StatusCode::CONFLICT);

    // Delete finalized file behind OfflineStore's back
    std::fs::remove_file(&final_path).unwrap();
    let fresh_url = proxy.stream_url_with_gen("heal_1", 140, proxy.current_generation());
    let healed_resp = client.get(&fresh_url).send().await.unwrap();
    assert_eq!(healed_resp.status(), StatusCode::OK);
    let healed_row = cache.get_download("heal_1").unwrap().unwrap();
    assert_eq!(healed_row.status, DownloadStatus::Failed);
}

#[tokio::test]
async fn playback_controller_skips_provider_resolution_for_completed_offline_track() {
    let payload = minimal_valid_m4a_bytes();
    let cdn = MockUpstreamCdn::start(payload.clone()).await;
    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let chunks = Arc::new(ChunkCache::new(dir.path(), cache.clone(), 64 * 1024 * 1024).unwrap());
    let artwork = Arc::new(ArtworkCache::new(dir.path(), 16, 16 * 1024 * 1024).unwrap());
    let provider = Arc::new(MockDownloadProvider::new(
        cdn.base_url.clone(),
        payload.len() as u64,
    ));
    let mgr = DownloadManager::new(
        provider.clone(),
        cache.clone(),
        chunks.clone(),
        artwork,
        Arc::new(MockLyrics { doc: None }),
        EventBus::new(64),
    )
    .unwrap();

    mgr.enqueue(sample_track("ctrl_off_1")).await.unwrap();
    mgr.enqueue(sample_track("ctrl_off_2")).await.unwrap();
    wait_for_status(&cache, "ctrl_off_1", |s| s == DownloadStatus::Completed).await;
    wait_for_status(&cache, "ctrl_off_2", |s| s == DownloadStatus::Completed).await;

    provider.resolve_calls.store(0, Ordering::SeqCst);

    let proxy = Arc::new(
        LoopbackProxy::start_with_offline(provider.clone(), chunks, Some(cache.clone()))
            .await
            .unwrap(),
    );
    let audio = Arc::new(MockAudioBackend::new());
    let lyrics = Arc::new(MockLyrics { doc: None });
    let sb = Arc::new(resonance_core::sponsorblock::HttpSponsorBlockProvider::new());
    let ctrl = PlaybackController::new(
        provider.clone(),
        audio,
        cache.clone(),
        proxy,
        lyrics,
        sb,
        EventBus::new(64),
    )
    .with_offline_store(cache.clone());

    ctrl.play_tracks(
        vec![sample_track("ctrl_off_1"), sample_track("ctrl_off_2")],
        0,
    )
    .await
    .unwrap();
    let snap = ctrl.snapshot().await;
    assert_eq!(snap.state, PlayerState::Playing);
    assert!(snap.offline_mode);
    assert_eq!(provider.resolve_calls.load(Ordering::SeqCst), 0);

    // Advance past 50% -> next track is also offline, so pre-resolution should be skipped
    let _ = ctrl.tick_position(100_000).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(provider.resolve_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn security_path_traversal_quota_and_secret_redaction_gates() {
    for bad in ["../etc/passwd", "a/b", "", &"x".repeat(65)] {
        let err = validate_video_id(bad).unwrap_err();
        assert!(matches!(
            err,
            CoreError::OfflineStorage(OfflineStorageError::InvalidVideoId(_))
        ));
    }

    let payload = minimal_valid_m4a_bytes();
    let cdn = MockUpstreamCdn::start(payload.clone()).await;
    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let chunks = Arc::new(ChunkCache::new(dir.path(), cache.clone(), 64 * 1024 * 1024).unwrap());
    let artwork = Arc::new(ArtworkCache::new(dir.path(), 16, 16 * 1024 * 1024).unwrap());
    let provider = Arc::new(MockDownloadProvider::new(
        cdn.base_url.clone(),
        payload.len() as u64,
    ));

    // LoopbackProxy rejects invalid video IDs with 400 Bad Request
    let proxy =
        LoopbackProxy::start_with_offline(provider.clone(), chunks.clone(), Some(cache.clone()))
            .await
            .unwrap();
    let bad_url = format!("http://{}/stream/a%2Fb?itag=140&gen=0", proxy.addr());
    let bad_resp = reqwest::get(&bad_url).await.unwrap();
    assert_eq!(bad_resp.status(), StatusCode::BAD_REQUEST);

    // Quota gate
    let tiny_mgr = DownloadManager::new(
        provider.clone(),
        cache.clone(),
        chunks.clone(),
        artwork.clone(),
        Arc::new(MockLyrics { doc: None }),
        EventBus::new(64),
    )
    .unwrap()
    .with_max_offline_bytes(100);
    tiny_mgr.enqueue(sample_track("quota_1")).await.unwrap();
    let q_rec = wait_for_status(&cache, "quota_1", |s| s == DownloadStatus::Failed).await;
    assert!(
        q_rec
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("quota")
            || q_rec.status == DownloadStatus::Failed
    );

    // Secret redaction gate
    provider.force_fail.store(true, Ordering::SeqCst);
    let bus = EventBus::new(64);
    let mut rx = bus.subscribe();
    let mgr = DownloadManager::new(
        provider,
        cache.clone(),
        chunks,
        artwork,
        Arc::new(MockLyrics { doc: None }),
        bus,
    )
    .unwrap();
    mgr.enqueue(sample_track("redact_1")).await.unwrap();
    let fail_rec = wait_for_status(&cache, "redact_1", |s| s == DownloadStatus::Failed).await;
    let err_msg = fail_rec.error_message.unwrap_or_default();
    for secret in [
        "SECRET_SIG",
        "SECRET_N",
        "SECRET_POT",
        "SECRET_COOKIE",
        "SAPISID=",
    ] {
        assert!(
            !err_msg.contains(secret),
            "secret {secret} leaked in {err_msg}"
        );
    }
    while let Ok(ev) = rx.try_recv() {
        let dbg = format!("{ev:?}");
        for secret in ["SECRET_SIG", "SECRET_N", "SECRET_POT", "SECRET_COOKIE"] {
            assert!(!dbg.contains(secret), "secret {secret} leaked in AppEvent");
        }
    }
}
