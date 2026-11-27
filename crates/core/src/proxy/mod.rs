//! Proxy v2: In-Process Loopback HTTP Streaming Proxy (`http://127.0.0.1:<port>/stream/<video_id>?itag=<itag>`).
//!
//! Features:
//! - Full `HTTP 206 Partial Content` byte-range serving for instant arbitrary seeking
//! - Dual-buffer slot engine (`current_track` + `next_track` pre-warmed at >= 75% progress)
//! - Range-coalescing & single-flight chunk deduplication (`ChunkCoalescer`)
//! - Serves directly from the 256 KB atomic `ChunkCache` (offline replay `<50ms`, SHA-256 verified)
//! - Priority 0 local-disk bypass for completed offline downloads (`X-Resonance-Source: offline-disk`)
//! - Transparent mid-stream `403` URL re-resolution (handles ~6h googlevideo TTL expiry
//!   and Wi-Fi <-> 5G handoffs without interrupting the audio player)

pub mod buffer;
pub mod coalesce;
pub mod lan_relay;
pub mod local_route;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use resonance_contract::{
    CoreError, CoreResult, DownloadStatus, MusicProvider, OfflineStore, PlaybackError, StreamInfo,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::cache::artwork::ArtworkCache;
use crate::cache::chunks::{CHUNK_SIZE, ChunkCache};
use crate::local_media::store::LocalMediaStore;
use crate::offline::{resolve_confined_path, validate_video_id};
use crate::unix_secs;

pub use buffer::{BufferSlot, DualBufferSlots, DualBufferStatus};
pub use coalesce::{AlignedWindow, ChunkCoalescer, ChunkKey, coalesce_byte_range};

/// Maximum contiguous aligned chunks fetched in a single coalesced upstream range request.
const MAX_COALESCED_WINDOW_CHUNKS: u64 = 4;

pub struct LoopbackProxy {
    addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    state: Arc<ProxyState>,
}

struct ProxyState {
    provider: Arc<dyn MusicProvider>,
    chunks: Arc<ChunkCache>,
    offline: Option<Arc<dyn OfflineStore>>,
    artwork: Option<Arc<ArtworkCache>>,
    local_store: RwLock<Option<Arc<LocalMediaStore>>>,
    http: reqwest::Client,
    slots: RwLock<DualBufferSlots>,
    coalescer: ChunkCoalescer,
    active_generation: AtomicU64,
}

impl LoopbackProxy {
    pub async fn start(
        provider: Arc<dyn MusicProvider>,
        chunks: Arc<ChunkCache>,
    ) -> CoreResult<Self> {
        Self::start_with_caches(provider, chunks, None, None).await
    }

    pub async fn start_with_offline(
        provider: Arc<dyn MusicProvider>,
        chunks: Arc<ChunkCache>,
        offline: Option<Arc<dyn OfflineStore>>,
    ) -> CoreResult<Self> {
        Self::start_with_caches(provider, chunks, offline, None).await
    }

    pub async fn start_with_caches(
        provider: Arc<dyn MusicProvider>,
        chunks: Arc<ChunkCache>,
        offline: Option<Arc<dyn OfflineStore>>,
        artwork: Option<Arc<ArtworkCache>>,
    ) -> CoreResult<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;
        let addr = listener
            .local_addr()
            .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;

        let state = Arc::new(ProxyState {
            provider,
            chunks,
            offline,
            artwork,
            local_store: RwLock::new(None),
            http,
            slots: RwLock::new(DualBufferSlots::default()),
            coalescer: ChunkCoalescer::new(),
            active_generation: AtomicU64::new(0),
        });

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let state_clone = state.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accept_res = listener.accept() => {
                        let Ok((stream, _)) = accept_res else { continue };
                        let io = TokioIo::new(stream);
                        let st = state_clone.clone();
                        tokio::spawn(async move {
                            let svc = service_fn(move |req| {
                                let st = st.clone();
                                async move { handle_request(req, st).await }
                            });
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(io, svc)
                                .await;
                        });
                    }
                }
            }
        });

        Ok(Self {
            addr,
            shutdown_tx: Some(shutdown_tx),
            state,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn set_local_media_store(&self, store: Arc<LocalMediaStore>) {
        *self.state.local_store.write().unwrap() = Some(store);
    }

    pub fn local_url(&self, track_id: &str) -> String {
        format!("http://{}/local/{track_id}", self.addr)
    }

    pub fn local_art_url(&self, art_hash: &str) -> String {
        format!("http://{}/local-art/{art_hash}", self.addr)
    }

    /// Registers a resolved stream so the proxy can immediately serve it without an extra resolve.
    pub fn register_stream(&self, stream: StreamInfo) {
        self.state.slots.write().unwrap().register(stream);
    }

    /// Explicitly binds a resolved stream to the `current_track` buffer slot (promoting warmed
    /// chunks if it was previously prepared in the `next_track` slot).
    pub fn register_current_stream(&self, stream: StreamInfo) {
        self.state.slots.write().unwrap().set_current(stream);
    }

    /// Explicitly binds an upcoming stream to the `next_track` dual-buffer slot.
    pub fn register_next_stream(&self, stream: StreamInfo) {
        self.state.slots.write().unwrap().set_next(stream);
    }

    /// Pre-warms the `next_track` slot by fetching up to `chunks_to_warm` aligned 256 KB
    /// chunks into `ChunkCache` through the single-flight `ChunkCoalescer`.
    /// Returns the number of chunks warmed on disk.
    pub async fn warm_next_track(
        &self,
        stream: StreamInfo,
        chunks_to_warm: u64,
    ) -> CoreResult<u64> {
        let video_id = stream.video_id.clone();
        let itag = stream.itag;
        let max_chunks = if stream.content_length > 0 {
            stream.content_length.div_ceil(CHUNK_SIZE)
        } else {
            1
        };
        let target_chunks = chunks_to_warm.clamp(1, max_chunks);
        self.register_next_stream(stream);

        let mut warmed = 0u64;
        for chunk_idx in 0..target_chunks {
            fetch_chunk_with_403_recovery(&self.state, &video_id, itag, chunk_idx).await?;
            warmed += 1;
            self.state
                .slots
                .write()
                .unwrap()
                .record_warmed_chunk(&video_id, warmed);
        }
        Ok(warmed)
    }

    /// Returns a snapshot of the `current_track` and `next_track` dual-buffer slots.
    pub fn dual_buffer_status(&self) -> DualBufferStatus {
        self.state.slots.read().unwrap().status()
    }

    /// Returns the number of concurrent chunk fetches coalesced via single-flight deduplication.
    pub fn coalesced_hits(&self) -> u64 {
        self.state.coalescer.coalesced_hits()
    }

    pub fn stream_url(&self, video_id: &str, itag: u32) -> String {
        let ext = if matches!(itag, 251 | 774) {
            ""
        } else {
            ".m4a"
        };
        format!("http://{}/stream/{video_id}{ext}?itag={itag}", self.addr)
    }

    /// Advances the monotonic playback generation ID, cancelling any in-flight
    /// proxy fetches tagged with an older generation (rapid track skips).
    pub fn advance_generation(&self) -> u64 {
        self.state.active_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn current_generation(&self) -> u64 {
        self.state.active_generation.load(Ordering::SeqCst)
    }

    pub fn stream_url_with_gen(&self, video_id: &str, itag: u32, generation: u64) -> String {
        let ext = if matches!(itag, 251 | 774) {
            ""
        } else {
            ".m4a"
        };
        format!(
            "http://{}/stream/{video_id}{ext}?itag={itag}&gen={generation}",
            self.addr
        )
    }
}

impl Drop for LoopbackProxy {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

fn parse_byte_range(header: &str, total_len: u64) -> Option<(u64, u64)> {
    let spec = header.strip_prefix("bytes=")?;
    let (start_s, end_s) = spec.split_once('-')?;
    let start: u64 = start_s.trim().parse().ok()?;
    if start >= total_len {
        return None;
    }
    let end = if end_s.trim().is_empty() {
        total_len.saturating_sub(1)
    } else {
        end_s
            .trim()
            .parse::<u64>()
            .ok()?
            .min(total_len.saturating_sub(1))
    };
    if end < start {
        return None;
    }
    Some((start, end))
}

fn detect_image_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else {
        "image/jpeg"
    }
}

async fn handle_artwork_request(
    req: Request<Incoming>,
    state: Arc<ProxyState>,
) -> Response<Full<Bytes>> {
    let mut target_url: Option<String> = None;
    if let Some(q) = req.uri().query() {
        for (k, v) in url::form_urlencoded::parse(q.as_bytes()) {
            if k == "url" {
                target_url = Some(v.into_owned());
                break;
            }
        }
    }
    let Some(raw_url) = target_url else {
        return simple_response(StatusCode::BAD_REQUEST, "missing url");
    };
    let normalized = if let Some(rest) = raw_url.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        raw_url
    };
    let Ok(parsed) = url::Url::parse(&normalized) else {
        return simple_response(StatusCode::BAD_REQUEST, "invalid artwork url");
    };
    if !matches!(parsed.scheme(), "https" | "http") {
        return simple_response(StatusCode::BAD_REQUEST, "unsupported scheme");
    }
    if matches!(
        parsed.host_str(),
        Some("127.0.0.1" | "localhost" | "::1" | "0.0.0.0")
    ) {
        return simple_response(StatusCode::FORBIDDEN, "loopback artwork forbidden");
    }

    if let Some(cache) = &state.artwork
        && let Some(bytes) = cache.get(&normalized, 0)
    {
        let mime = detect_image_mime(&bytes);
        return Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, mime)
            .header(CONTENT_LENGTH, bytes.len())
            .header("Cache-Control", "public, max-age=86400")
            .body(Full::new(Bytes::from(bytes)))
            .unwrap();
    }

    let mut last_status = StatusCode::BAD_GATEWAY;
    for attempt in 0..3u32 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(120 * u64::from(attempt))).await;
        }
        let resp = state
            .http
            .get(normalized.as_str())
            .header("Accept", "image/jpeg,image/png,image/webp,*/*;q=0.8")
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
            )
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                if let Ok(bytes) = r.bytes().await
                    && !bytes.is_empty()
                {
                    let vec = bytes.to_vec();
                    let mime = detect_image_mime(&vec);
                    if let Some(cache) = &state.artwork {
                        let _ = cache.put(&normalized, 0, vec.clone());
                    }
                    return Response::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, mime)
                        .header(CONTENT_LENGTH, vec.len())
                        .header("Cache-Control", "public, max-age=86400")
                        .body(Full::new(Bytes::from(vec)))
                        .unwrap();
                }
            }
            Ok(r) => {
                last_status =
                    StatusCode::from_u16(r.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                if r.status().is_client_error() && r.status().as_u16() != 429 {
                    if let Some(fallback) = normalized.strip_suffix("=w544-h544-l90-rj") {
                        let smaller = format!("{fallback}=w226-h226-l90-rj");
                        if let Ok(fr) = state.http.get(&smaller).send().await
                            && fr.status().is_success()
                            && let Ok(bytes) = fr.bytes().await
                            && !bytes.is_empty()
                        {
                            let vec = bytes.to_vec();
                            let mime = detect_image_mime(&vec);
                            if let Some(cache) = &state.artwork {
                                let _ = cache.put(&normalized, 0, vec.clone());
                            }
                            return Response::builder()
                                .status(StatusCode::OK)
                                .header(CONTENT_TYPE, mime)
                                .header(CONTENT_LENGTH, vec.len())
                                .header("Cache-Control", "public, max-age=86400")
                                .body(Full::new(Bytes::from(vec)))
                                .unwrap();
                        }
                    }
                    break;
                }
            }
            Err(_) => {
                last_status = StatusCode::BAD_GATEWAY;
            }
        }
    }

    simple_response(last_status, "artwork fetch failed")
}

async fn handle_request(
    req: Request<Incoming>,
    state: Arc<ProxyState>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let path = req.uri().path();
    if path == "/artwork" {
        return Ok(handle_artwork_request(req, state).await);
    }
    if path.starts_with("/local/") || path.starts_with("/local-art/") {
        let local_store = state.local_store.read().unwrap().clone();
        if let Some(resp) =
            local_route::handle_local_request(&req, local_store.as_ref(), state.artwork.as_ref())
                .await
        {
            return Ok(resp);
        }
    }
    let Some(video_id_raw) = path.strip_prefix("/stream/") else {
        return Ok(simple_response(StatusCode::NOT_FOUND, "not found"));
    };
    let video_id = video_id_raw.strip_suffix(".m4a").unwrap_or(video_id_raw);
    if validate_video_id(video_id).is_err() {
        return Ok(simple_response(StatusCode::BAD_REQUEST, "invalid video_id"));
    }

    let mut requested_itag: u32 = 251;
    let mut request_gen: Option<u64> = None;
    if let Some(q) = req.uri().query() {
        for (k, v) in url::form_urlencoded::parse(q.as_bytes()) {
            if k == "itag" {
                if let Ok(parsed) = v.parse::<u32>() {
                    requested_itag = parsed;
                }
            } else if k == "gen" {
                request_gen = v.parse::<u64>().ok();
            }
        }
    }

    // Monotonic generation-ID cancellation: immediately reject superseded track skips.
    if let Some(g) = request_gen
        && g < state.active_generation.load(Ordering::SeqCst)
    {
        return Ok(simple_response(
            StatusCode::CONFLICT,
            "cancelled: superseded track generation",
        ));
    }

    let is_head = *req.method() == hyper::Method::HEAD;
    let range_header = req
        .headers()
        .get(RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    // Priority 0: Local-Disk Bypass for completed offline downloads (zero network I/O).
    if let Some(resp) =
        try_serve_offline_file(&state, video_id, range_header.as_deref(), request_gen, is_head)
            .await
    {
        return Ok(resp);
    }

    // Determine content length & mime type from chunk cache first (enables instant offline replay!)
    let mut stream_info = state.slots.read().unwrap().get(video_id);

    let known_cached_len = state
        .chunks
        .known_total_len(video_id, requested_itag)
        .ok()
        .flatten();

    let (total_len, mime_type, itag) = if let Some(len) = known_cached_len {
        let mime = stream_info
            .as_ref()
            .map(|s| s.mime_type.clone())
            .unwrap_or_else(|| {
                if matches!(requested_itag, 251 | 774) {
                    "audio/webm".to_owned()
                } else {
                    "audio/mp4".to_owned()
                }
            });
        (len, mime, requested_itag)
    } else {
        if stream_info.is_none() {
            match state.provider.resolve_stream(video_id).await {
                Ok(info) => {
                    state.slots.write().unwrap().register(info.clone());
                    stream_info = Some(info);
                }
                Err(e) => {
                    return Ok(simple_response(
                        StatusCode::BAD_GATEWAY,
                        &format!("resolve failed: {e}"),
                    ));
                }
            }
        }
        let Some(info) = stream_info.as_ref() else {
            return Ok(simple_response(
                StatusCode::BAD_GATEWAY,
                "missing stream info",
            ));
        };
        let len = if info.content_length > 0 {
            info.content_length
        } else {
            match fetch_chunk_with_403_recovery(&state, video_id, info.itag, 0).await {
                Ok((_, detected_total)) if detected_total > 0 => detected_total,
                Ok((bytes, _)) => bytes.len() as u64,
                Err(e) => {
                    return Ok(simple_response(
                        StatusCode::BAD_GATEWAY,
                        &format!("upstream error: {e}"),
                    ));
                }
            }
        };
        (len, info.mime_type.clone(), info.itag)
    };

    if total_len == 0 {
        return Ok(simple_response(StatusCode::BAD_GATEWAY, "empty stream"));
    }

    let clean_mime = mime_type
        .split(';')
        .next()
        .unwrap_or("audio/mp4")
        .trim()
        .to_owned();

    let (req_start, req_end, is_partial) = match range_header.as_deref() {
        Some(hdr) => match parse_byte_range(hdr, total_len) {
            Some((s, e)) => (s, e, true),
            None => {
                return Ok(Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(CONTENT_RANGE, format!("bytes */{total_len}"))
                    .body(Full::new(Bytes::new()))
                    .unwrap());
            }
        },
        None => (0, total_len.saturating_sub(1), false),
    };

    if is_head {
        let mut builder = Response::builder()
            .status(if is_partial {
                StatusCode::PARTIAL_CONTENT
            } else {
                StatusCode::OK
            })
            .header(ACCEPT_RANGES, "bytes")
            .header(CONTENT_TYPE, &clean_mime)
            .header(CONTENT_LENGTH, req_end - req_start + 1);
        if is_partial {
            builder = builder.header(
                CONTENT_RANGE,
                format!("bytes {req_start}-{req_end}/{total_len}"),
            );
        }
        return Ok(builder.body(Full::new(Bytes::new())).unwrap());
    }

    let effective_end = req_end;
    let windows = coalesce_byte_range(
        req_start,
        effective_end,
        total_len,
        MAX_COALESCED_WINDOW_CHUNKS,
    );
    let mut out = Vec::with_capacity((effective_end - req_start + 1) as usize);

    for win in windows {
        for chunk_idx in win.first_chunk..=win.last_chunk {
            if let Some(g) = request_gen
                && g < state.active_generation.load(Ordering::SeqCst)
            {
                return Ok(simple_response(
                    StatusCode::CONFLICT,
                    "cancelled: superseded track generation",
                ));
            }
            let chunk_bytes =
                match fetch_chunk_with_403_recovery(&state, video_id, itag, chunk_idx).await {
                    Ok((bytes, _)) => bytes,
                    Err(e) => {
                        return Ok(simple_response(
                            StatusCode::BAD_GATEWAY,
                            &format!("chunk fetch failed: {e}"),
                        ));
                    }
                };

            let chunk_global_start = chunk_idx * CHUNK_SIZE;
            let slice_start = req_start.saturating_sub(chunk_global_start) as usize;
            let slice_end = ((effective_end + 1).saturating_sub(chunk_global_start) as usize)
                .min(chunk_bytes.len());

            if slice_start < slice_end {
                out.extend_from_slice(&chunk_bytes[slice_start..slice_end]);
            }
        }
    }

    let actual_end = req_start + (out.len() as u64).saturating_sub(1);
    let mut builder = Response::builder()
        .header(CONTENT_TYPE, &clean_mime)
        .header(ACCEPT_RANGES, "bytes")
        .header(CONTENT_LENGTH, out.len().to_string());

    if is_partial || actual_end < total_len - 1 {
        builder = builder.status(StatusCode::PARTIAL_CONTENT).header(
            CONTENT_RANGE,
            format!("bytes {req_start}-{actual_end}/{total_len}"),
        );
    } else {
        builder = builder.status(StatusCode::OK);
    }

    Ok(builder
        .body(Full::new(Bytes::from(out)))
        .unwrap_or_else(|_| {
            simple_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "invalid proxy response headers",
            )
        }))
}

/// Fetches a 256 KB chunk from `ChunkCache` first; on cache miss collapses concurrent
/// callers through `ChunkCoalescer` into a single upstream fetch.
/// If upstream returns HTTP `403 Forbidden` (expired ~6h URL or Wi-Fi/5G IP handoff),
/// transparently re-resolves the stream via `MusicProvider::resolve_stream` and retries.
async fn fetch_chunk_with_403_recovery(
    state: &ProxyState,
    video_id: &str,
    itag: u32,
    chunk_index: u64,
) -> CoreResult<(Vec<u8>, u64)> {
    // Tier 1 Cache Hit: instant offline replay (< 50ms) with SHA-256 verification.
    if let Ok(Some((bytes, total_len))) = state.chunks.read_chunk(video_id, itag, chunk_index) {
        return Ok((bytes, total_len));
    }

    let key = ChunkKey {
        video_id: video_id.to_owned(),
        itag,
        chunk_index,
    };

    state
        .coalescer
        .fetch_or_join(key, || async move {
            // Re-check cache inside single-flight leader in case another caller just committed.
            if let Ok(Some((bytes, total_len))) =
                state.chunks.read_chunk(video_id, itag, chunk_index)
            {
                return Ok((bytes, total_len));
            }

            for attempt in 0..2 {
                let info = {
                    let cached = state.slots.read().unwrap().get(video_id);
                    match cached {
                        Some(s) if attempt == 0 && s.itag == itag => s,
                        _ => {
                            let fresh = state.provider.resolve_stream(video_id).await?;
                            state.slots.write().unwrap().register(fresh.clone());
                            if fresh.itag != itag {
                                return Err(CoreError::Playback(PlaybackError::StreamExpired(
                                    format!(
                                        "re-resolved stream itag changed ({itag} -> {}); reload required",
                                        fresh.itag
                                    ),
                                )));
                            }
                            fresh
                        }
                    }
                };

                let range_start = chunk_index * CHUNK_SIZE;
                let range_end = if info.content_length > 0 {
                    (range_start + CHUNK_SIZE - 1).min(info.content_length.saturating_sub(1))
                } else {
                    range_start + CHUNK_SIZE - 1
                };

                let resp = state
                    .http
                    .get(&info.url)
                    .header(RANGE, format!("bytes={range_start}-{range_end}"))
                    .send()
                    .await
                    .map_err(|e| CoreError::Offline(format!("upstream fetch failed: {e}")))?;

                if resp.status() == reqwest::StatusCode::FORBIDDEN && attempt == 0 {
                    log::warn!(
                        "loopback proxy caught HTTP 403 mid-stream for {video_id} chunk {chunk_index}; transparently re-resolving"
                    );
                    state.slots.write().unwrap().remove(video_id);
                    continue;
                }

                if !resp.status().is_success() {
                    return Err(CoreError::Playback(PlaybackError::StreamExpired(format!(
                        "HTTP {} on chunk {chunk_index}",
                        resp.status()
                    ))));
                }

                let detected_total = resp
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.rsplit_once('/'))
                    .and_then(|(_, tot)| tot.parse::<u64>().ok())
                    .unwrap_or(info.content_length);

                let bytes = resp
                    .bytes()
                    .await
                    .map_err(|e| CoreError::Offline(format!("body read failed: {e}")))?
                    .to_vec();

                let final_total = if detected_total > 0 {
                    detected_total
                } else {
                    range_start + bytes.len() as u64
                };

                // Atomically persist chunk with SHA-256 integrity hash & LRU eviction.
                let _ = state
                    .chunks
                    .write_chunk(video_id, itag, chunk_index, &bytes, final_total);

                return Ok((bytes, final_total));
            }

            Err(CoreError::Playback(PlaybackError::StreamExpired(format!(
                "failed to recover 403 for {video_id}"
            ))))
        })
        .await
}

async fn try_serve_offline_file(
    state: &ProxyState,
    video_id: &str,
    range_header: Option<&str>,
    request_gen: Option<u64>,
    is_head: bool,
) -> Option<Response<Full<Bytes>>> {
    let offline = state.offline.as_ref()?;
    let rec = offline.get_completed_download(video_id).ok().flatten()?;
    let rel_path = rec.relative_path.as_deref()?;

    let Ok(path) = resolve_confined_path(&offline.downloads_root(), rel_path) else {
        log::warn!("offline file path rejected for {video_id}");
        let _ = offline.mark_download_failed(
            video_id,
            DownloadStatus::Failed,
            Some("offline file missing or truncated"),
            unix_secs(),
        );
        return None;
    };

    let Ok(mut file) = tokio::fs::File::open(&path).await else {
        log::warn!("offline file missing on disk for {video_id}; self-healing to fallback");
        let _ = offline.mark_download_failed(
            video_id,
            DownloadStatus::Failed,
            Some("offline file missing or truncated"),
            unix_secs(),
        );
        return None;
    };

    let file_len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    if file_len == 0 || (rec.total_bytes > 0 && file_len != rec.total_bytes) {
        log::warn!(
            "offline file truncated for {video_id} (disk={file_len}, expected={}); self-healing",
            rec.total_bytes
        );
        let _ = offline.mark_download_failed(
            video_id,
            DownloadStatus::Failed,
            Some("offline file missing or truncated"),
            unix_secs(),
        );
        return None;
    }

    let (req_start, req_end, is_partial) = match range_header {
        Some(hdr) => match parse_byte_range(hdr, file_len) {
            Some((s, e)) => (s, e, true),
            None => {
                return Some(
                    Response::builder()
                        .status(StatusCode::RANGE_NOT_SATISFIABLE)
                        .header(CONTENT_RANGE, format!("bytes */{file_len}"))
                        .body(Full::new(Bytes::new()))
                        .unwrap(),
                );
            }
        },
        None => (0, file_len.saturating_sub(1), false),
    };

    if is_head {
        let mut builder = Response::builder()
            .status(if is_partial {
                StatusCode::PARTIAL_CONTENT
            } else {
                StatusCode::OK
            })
            .header(CONTENT_TYPE, rec.mime_type)
            .header(ACCEPT_RANGES, "bytes")
            .header(CONTENT_LENGTH, (req_end - req_start + 1).to_string())
            .header("X-Resonance-Source", "offline-disk");
        if is_partial {
            builder = builder.header(
                CONTENT_RANGE,
                format!("bytes {req_start}-{req_end}/{file_len}"),
            );
        }
        return Some(builder.body(Full::new(Bytes::new())).unwrap());
    }

    if file
        .seek(std::io::SeekFrom::Start(req_start))
        .await
        .is_err()
    {
        return None;
    }

    let total_to_read = (req_end - req_start + 1) as usize;
    let mut out = Vec::with_capacity(total_to_read);
    let mut buf = vec![0u8; CHUNK_SIZE as usize];
    let mut remaining = total_to_read;

    while remaining > 0 {
        if let Some(g) = request_gen
            && g < state.active_generation.load(Ordering::SeqCst)
        {
            return Some(simple_response(
                StatusCode::CONFLICT,
                "cancelled: superseded track generation",
            ));
        }
        let to_read = remaining.min(buf.len());
        match file.read(&mut buf[..to_read]).await {
            Ok(0) => break,
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                remaining -= n;
            }
            Err(_) => return None,
        }
    }

    let actual_end = req_start + (out.len() as u64).saturating_sub(1);
    let mut builder = Response::builder()
        .header(CONTENT_TYPE, rec.mime_type)
        .header(ACCEPT_RANGES, "bytes")
        .header(CONTENT_LENGTH, out.len().to_string())
        .header("X-Resonance-Source", "offline-disk");

    if is_partial || actual_end < file_len - 1 {
        builder = builder.status(StatusCode::PARTIAL_CONTENT).header(
            CONTENT_RANGE,
            format!("bytes {req_start}-{actual_end}/{file_len}"),
        );
    } else {
        builder = builder.status(StatusCode::OK);
    }

    Some(
        builder
            .body(Full::new(Bytes::from(out)))
            .unwrap_or_else(|_| {
                simple_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "invalid proxy response headers",
                )
            }),
    )
}

fn simple_response(status: StatusCode, msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from(msg.to_owned())))
        .unwrap()
}
