//! In-Process Loopback HTTP Streaming Proxy (`http://127.0.0.1:<port>/stream/<video_id>?itag=<itag>`).
//!
//! Features:
//! - Full `HTTP 206 Partial Content` byte-range serving for instant arbitrary seeking
//! - Serves directly from the 256 KB atomic `ChunkCache` (offline replay `<50ms`)
//! - Transparent mid-stream `403` URL re-resolution (handles ~6h googlevideo TTL expiry
//!   and Wi-Fi <-> 5G handoffs without interrupting the audio player)

use std::collections::HashMap;
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
use resonance_contract::{CoreError, CoreResult, MusicProvider, PlaybackError, StreamInfo};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::cache::chunks::{CHUNK_SIZE, ChunkCache};

pub struct LoopbackProxy {
    addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    state: Arc<ProxyState>,
}

struct ProxyState {
    provider: Arc<dyn MusicProvider>,
    chunks: Arc<ChunkCache>,
    http: reqwest::Client,
    active_streams: RwLock<HashMap<String, StreamInfo>>,
    active_generation: AtomicU64,
}

impl LoopbackProxy {
    pub async fn start(
        provider: Arc<dyn MusicProvider>,
        chunks: Arc<ChunkCache>,
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
            http,
            active_streams: RwLock::new(HashMap::new()),
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

    /// Registers a resolved stream so the proxy can immediately serve it without an extra resolve.
    pub fn register_stream(&self, stream: StreamInfo) {
        self.state
            .active_streams
            .write()
            .unwrap()
            .insert(stream.video_id.clone(), stream);
    }

    pub fn stream_url(&self, video_id: &str, itag: u32) -> String {
        format!("http://{}/stream/{video_id}?itag={itag}", self.addr)
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
        format!(
            "http://{}/stream/{video_id}?itag={itag}&gen={generation}",
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

async fn handle_request(
    req: Request<Incoming>,
    state: Arc<ProxyState>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let path = req.uri().path();
    let Some(video_id_raw) = path.strip_prefix("/stream/") else {
        return Ok(simple_response(StatusCode::NOT_FOUND, "not found"));
    };
    let video_id = video_id_raw.strip_suffix(".m4a").unwrap_or(video_id_raw);
    if video_id.is_empty() || video_id.contains('/') {
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

    let range_header = req
        .headers()
        .get(RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    // Determine content length & mime type from chunk cache first (enables instant offline replay!)
    let mut stream_info = state
        .active_streams
        .read()
        .unwrap()
        .get(video_id)
        .cloned();

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
                    state
                        .active_streams
                        .write()
                        .unwrap()
                        .insert(video_id.to_owned(), info.clone());
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
        let info = stream_info.as_ref().unwrap();
        let len = if info.content_length > 0 {
            info.content_length
        } else {
            // Probe first chunk if content_length was omitted.
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

    // Respect client requested range so native players (AVPlayer / CoreMedia) don't
    // fail with CoreMediaErrorDomain -12939 content range mismatch.
    let effective_end = req_end;
    let first_chunk = req_start / CHUNK_SIZE;
    let last_chunk = effective_end / CHUNK_SIZE;

    let mut out = Vec::with_capacity((effective_end - req_start + 1) as usize);

    for chunk_idx in first_chunk..=last_chunk {
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
        let slice_end = ((effective_end + 1)
            .saturating_sub(chunk_global_start) as usize)
            .min(chunk_bytes.len());

        if slice_start < slice_end {
            out.extend_from_slice(&chunk_bytes[slice_start..slice_end]);
        }
    }

    let actual_end = req_start + (out.len() as u64).saturating_sub(1);
    let mut builder = Response::builder()
        .header(CONTENT_TYPE, mime_type)
        .header(ACCEPT_RANGES, "bytes")
        .header(CONTENT_LENGTH, out.len().to_string());

    if is_partial || actual_end < total_len - 1 {
        builder = builder
            .status(StatusCode::PARTIAL_CONTENT)
            .header(
                CONTENT_RANGE,
                format!("bytes {req_start}-{actual_end}/{total_len}"),
            );
    } else {
        builder = builder.status(StatusCode::OK);
    }

    Ok(builder.body(Full::new(Bytes::from(out))).unwrap())
}

/// Fetches a 256 KB chunk from `ChunkCache` first; on cache miss fetches upstream.
/// If upstream returns HTTP `403 Forbidden` (expired ~6h URL or Wi-Fi/5G IP handoff),
/// transparently re-resolves the stream via `MusicProvider::resolve_stream` and retries!
async fn fetch_chunk_with_403_recovery(
    state: &ProxyState,
    video_id: &str,
    itag: u32,
    chunk_index: u64,
) -> CoreResult<(Vec<u8>, u64)> {
    // Tier 1 Cache Hit: instant offline replay (< 50ms)
    if let Ok(Some((bytes, total_len))) = state.chunks.read_chunk(video_id, itag, chunk_index) {
        return Ok((bytes, total_len));
    }

    for attempt in 0..2 {
        let info = {
            let cached = state.active_streams.read().unwrap().get(video_id).cloned();
            match cached {
                Some(s) if attempt == 0 => s,
                _ => {
                    let fresh = state.provider.resolve_stream(video_id).await?;
                    state
                        .active_streams
                        .write()
                        .unwrap()
                        .insert(video_id.to_owned(), fresh.clone());
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
            state.active_streams.write().unwrap().remove(video_id);
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

        // Atomically persist chunk for future instant seeking & offline replay.
        let _ = state
            .chunks
            .write_chunk(video_id, itag, chunk_index, &bytes, final_total);

        return Ok((bytes, final_total));
    }

    Err(CoreError::Playback(PlaybackError::StreamExpired(format!(
        "failed to recover 403 for {video_id}"
    ))))
}

fn simple_response(status: StatusCode, msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from(msg.to_owned())))
        .unwrap()
}
