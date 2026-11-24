# SPEC D — Authenticated InnerTube Write Mutations (`/atr`, `/like/like`, `/browse/edit_playlist`)

> Parallelism boundary: owns `crates/contract/src/mutations.rs`, `crates/core/src/innertube/mutations.rs`, and non-blocking Tokio queue wiring in `src/live_engine.rs` / `src/store.rs`. Zero edits to `platform/desktop.rs`, `cache/db.rs`, or `crates/core/src/potoken/`.

## 1. Exact `crates/contract` Signatures

### `crates/contract/src/mutations.rs` (new, re-exported in `lib.rs`)
```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use crate::TrackId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LikeAction {
    Like,
    Dislike,
    RemoveLike,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaylistEditOp {
    AddVideo { video_id: TrackId },
    RemoveVideo { video_id: TrackId, set_video_id: String },
    MoveVideoBefore { set_video_id: String, predecessor_set_video_id: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchtimeReport {
    pub video_id: TrackId,
    /// Client Playback Nonce (16-char base64url) stable per playback session.
    pub cpn: String,
    pub start_secs: u64,
    pub end_secs: u64,
    pub duration_secs: u64,
    pub is_final: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MutationCommand {
    ReportAtr(WatchtimeReport),
    SetLike { video_id: TrackId, action: LikeAction },
    EditPlaylist { playlist_id: String, op: PlaylistEditOp },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationReceipt {
    pub idempotency_key: String,
    pub endpoint: String,
    pub server_status: Option<String>,
    pub executed_at_unix_secs: u64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MutationError {
    #[error("authentication required for write mutation")]
    NotAuthenticated,
    #[error("invalid mutation payload: {0}")]
    InvalidInput(String),
    #[error("rate limited (429), retry after {retry_after_secs:?}s")]
    RateLimited { retry_after_secs: Option<u64> },
    #[error("mutation queue full (capacity {capacity})")]
    QueueFull { capacity: usize },
    #[error("http error {status}: {message}")]
    Http { status: u16, message: String },
    #[error("network error: {0}")]
    Network(String),
}

#[async_trait]
pub trait InnerTubeMutations: Send + Sync {
    async fn report_atr(&self, report: &WatchtimeReport) -> Result<MutationReceipt, MutationError>;
    async fn set_like(&self, video_id: &str, action: LikeAction) -> Result<MutationReceipt, MutationError>;
    async fn edit_playlist(&self, playlist_id: &str, op: &PlaylistEditOp) -> Result<MutationReceipt, MutationError>;
}
```
### `crates/contract/src/error.rs` & `events.rs` (additive variants)
```rust
// CoreError:
    #[error("mutation error: {0}")]
    Mutation(#[from] MutationError),
// AppEvent:
    MutationCompleted(MutationReceipt),
    MutationFailed { command: MutationCommand, reason: String },
```

## 2. File-by-File Implementation Plan

- **`crates/core/src/innertube/client.rs`** (minimal 10-line visibility seam):
  - Expose `pub(crate) async fn post_authenticated_web_remix(&self, endpoint: &str, payload: Value) -> CoreResult<Value>`:
    - Rejects immediately with `CoreError::Auth(AuthError::NotSignedIn)` if `self.current_session().is_none()`.
    - Always targets `&WEB_REMIX` (`AuthMode::CookiesOrAnonymous`), invoking `post_innertube(&WEB_REMIX, endpoint, payload)` which attaches `Cookie`, `Authorization: SAPISIDHASH ...`, `X-Goog-AuthUser`, optional `X-Goog-PageId`, and processes `Set-Cookie` rotation + `401` revocation.
  - Expose `pub(crate) fn web_remix_base_url(&self) -> String` (respects `api_base_override` for offline tests).
- **`crates/core/src/innertube/mutations.rs` (new, exported in `crates/core/src/innertube/mod.rs`)**:
  - `impl InnerTubeMutations for InnerTubeClient`:
    - **`set_like`**: maps `LikeAction::{Like, Dislike, RemoveLike}` -> endpoints `"like/like"`, `"like/dislike"`, `"like/removelike"` with payload `{"target": {"videoId": video_id}}`. Evicts SWR cache key `"collection:FEmusic_liked_videos"` on success.
    - **`edit_playlist`**: targets `"browse/edit_playlist"` with `{"playlistId": playlist_id, "actions": [...]}` (`ACTION_ADD_VIDEO` + `addedVideoId`; `ACTION_REMOVE_VIDEO` + `removedVideoId` + `setVideoId`; `ACTION_MOVE_VIDEO_BEFORE` + `setVideoId` + optional `movedSetVideoIdPredecessor`). Evicts SWR cache key `format!("collection:{playlist_id}")` on success.
    - **`report_atr`**: validates `video_id`, `cpn` (16 chars), and `end_secs >= start_secs`. Sends signed `GET {origin}/api/stats/atr` (or `{api_base_override}/atr` in tests) with query params `docid`, `cpn`, `ver=2`, `st`, `et`, `len`, `el=detailpage`, `ns=yt`, `c=WEB_REMIX`, `cver=WEB_REMIX.client_version`, attaching `build_auth_headers(&WEB_REMIX, &session, unix_secs())` (`SAPISIDHASH` + `Cookie`) + `X-Goog-Visitor-Id`.
  - **`MutationWorker` (non-blocking background queue)**:
    - Bounded `tokio::sync::mpsc::channel::<MutationCommand>(64)` (`try_send` returns `MutationError::QueueFull` so callers never block).
    - Single sequential consumer task preserving FIFO order per user (prevents `Like` + `RemoveLike` reordering) and coalescing redundant `ReportAtr` pings for the same `(video_id, cpn)` by keeping only the latest `end_secs`.
    - Retry policy: transient 5xx/network retries up to 3x (`500ms`, `1.5s`, `3s`); `429` honors `Retry-After` clamped `2..=30s`; `401` stops queue immediately, clears session via `InnerTubeClient::sign_out()`, emits `AppEvent::AuthStateChanged { signed_in: false, .. }` + `AppEvent::MutationFailed`.
- **`src/live_engine.rs` & `src/store.rs`**:
  - Instantiate `MutationWorker::spawn(provider.clone(), bus.clone())` inside `init_runtime()` on background Tokio runtime.
  - Add non-blocking dispatchers in `src/live_engine.rs`: `dispatch_set_like(TrackId, LikeAction)`, `dispatch_edit_playlist(String, PlaylistEditOp)`, `dispatch_report_atr(WatchtimeReport)`.
  - Hook `src/store.rs::toggle_like_current()` / `toggle_like_track()` / `toggle_dislike_current()` to optimistically update local UI signal and dispatch `live_engine::dispatch_set_like(...)` when `!is_fixture_mode() && signed_in`.
  - On `LiveUpdate::MutationFailed` for a `SetLike`, revert optimistic `is_liked` state and set `status_banner` without blocking UI thread.

## 3. Security & Concurrency Edge Cases

- **Strict `AuthMode` & anonymous isolation**: Write mutations are hardcoded to `&WEB_REMIX` (`AuthMode::CookiesOrAnonymous`) and rejected pre-flight if `current_session().is_none()`; `ANDROID_VR` and `IOS` (`AnonymousOnly`) are never touched or supplied cookies.
- **Session revocation (`401`)**: Any `401` on `/atr`, `/like/*`, or `/browse/edit_playlist` triggers `sign_out()`, drops queued pending authenticated mutations (preventing retry storms with revoked cookies), and notifies UI.
- **Rapid toggle race (`Like` -> `RemoveLike` -> `Like`)**: Single FIFO worker guarantees server observes transitions in exact click order; optimistic UI rollback only reverts if no newer local generation superseded that `video_id`.
- **Track switch during `/atr` reporting**: Final `/atr` report (`is_final: true`) captures `cpn` + `end_secs` snapshot before track switch clears `current_track`, queued asynchronously so skipping tracks has zero latency penalty.
- **Input validation & secret redaction**: Validate `video_id` and `playlist_id` against `[A-Za-z0-9_-]{1,128}` before building URLs/JSON; pass HTTP error bodies through `redact_secrets()` and cap at 200 chars so `Cookie`/`SAPISIDHASH` never surface in `MutationError` or `AppEvent::MutationFailed`.

## 4. Offline Verification Tests (`tests/innertube_mutations.rs`)

Mocks: local `127.0.0.1:0` `hyper` InnerTube test server recording method, path, query, `Authorization`, `Cookie`, `Origin`, `X-Goog-AuthUser`, and JSON body; configurable status scripts (`200`, `401`, `429`, `500`).
1. `mutations_reject_anonymous_session_without_making_network_calls` — anonymous `InnerTubeClient` calls `set_like`, `edit_playlist`, `report_atr`; asserts `Err(MutationError::NotAuthenticated)` and mock server request count == 0.
2. `set_like_signs_sapisidhash_hits_exact_endpoints_and_evicts_liked_cache` — verifies `Like` (`/like/like`), `Dislike` (`/like/dislike`), `RemoveLike` (`/like/removelike`) send valid `SAPISIDHASH` for `https://music.youtube.com`, `Cookie`, `X-Goog-AuthUser: 0`, exact `target.videoId`, and evict `collection:FEmusic_liked_videos`.
3. `edit_playlist_add_remove_move_payloads_and_cache_invalidation` — asserts exact JSON `actions` schemas for `AddVideo`, `RemoveVideo`, `MoveVideoBefore` on `/browse/edit_playlist` and SWR cache eviction for `collection:{playlist_id}`.
4. `report_atr_sends_signed_watchtime_params_and_rejects_invalid_ranges` — asserts `/atr` carries `SAPISIDHASH`, `docid`, `cpn`, `st`, `et`, `len`, `c=WEB_REMIX`; rejects `end_secs < start_secs` with `MutationError::InvalidInput`.
5. `mutation_worker_preserves_fifo_order_and_coalesces_atr_pings` — enqueues `Like` -> `RemoveLike` -> `Like` and 5 rapid `ReportAtr` pings for same `(video_id, cpn)`; asserts server receives likes in exact order and coalesced `/atr` reports.
6. `mutation_401_revokes_session_and_drains_pending_queue` — scripts `401` on first queued mutation; asserts `current_session().is_none()`, `AppEvent::MutationFailed` emitted, and subsequent queued mutations are not sent.
7. `mutation_429_backoff_and_secret_redaction` — scripts `429` (`Retry-After: 0`) -> `200` (succeeds); scripts `500` containing `SAPISID=LEAK` -> asserts redacted error string never contains `LEAK`.

```bash
cargo test --test innertube_mutations -- --nocapture
cargo test --test architecture_gates_v2 --test core_unit_and_snapshots
```
