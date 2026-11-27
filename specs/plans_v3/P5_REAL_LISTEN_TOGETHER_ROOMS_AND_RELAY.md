# P5: Real Listen Together Rooms & Hardened Room Relay Server

> **Wave:** B (Parallel, after Stitch Gate A)
> **Status of Existing Code:** `crates/core/src/rooms/*` (`clock_sync.rs`, `controller.rs`, `drift.rs`, `long_poll.rs`, `protocol.rs`, `ws_client.rs`), `crates/room-relay/src/*`, `src/store/rooms_slice.rs`, and `src/screens/listen_together.rs` exist, **but**:
> 1. `rooms_slice.rs` / `app_actions.rs` was previously faking local room creation without opening a WebSocket/long-poll connection to `room-relay` (we just made it honest — now wire the real networked engine!).
> 2. `crates/room-relay` needs production hardening (per-IP token-bucket rate limiting, max room capacity, idle room TTL eviction, graceful host failover on disconnect, room invite secret validation, and TLS/WSS reverse-proxy readiness).
> 3. `RoomController` drift correction (`soft rate 0.98x–1.02x` vs `hard seek > 500ms`) needs wiring directly into `PlaybackController` + `P1`'s `PcmAudioEngine` playback rate!
> **Exclusive Owned Paths:** `crates/core/src/rooms/*`, `crates/room-relay/*`, `src/store/rooms_slice.rs`, `src/screens/listen_together.rs`, `src/live_engine/handlers/rooms.rs`, `tests/p5_rooms_relay_test.rs`.

---

## 1. Deep Technical Research & Architecture

### 1.1 Synchronized Playback Math (NTP Clock Sync + Dual-Zone Drift Controller)
Synchronizing audio across residential networks (20–180ms jitter) requires separating **clock offset** from **media playback drift**:

1. **Clock Sync (`clock_sync.rs`):**
   - Client sends `Ping { client_send_ms: t1 }` every 10 seconds (burst of 5 pings on initial room join).
   - Server responds immediately with `Pong { client_send_ms: t1, server_recv_ms: t2, server_send_ms: t3 }`.
   - Client records `t4`, computes:
     - `rtt = (t4 - t1) - (t3 - t2)`
     - `offset = ((t2 - t1) + (t3 - t4)) / 2`
   - Maintain a sliding window of the last 8 samples, **discarding the top 2 highest-RTT outliers** (asymmetric cellular/Wi-Fi queue spikes), and take the **median offset**.
   - Estimated server time at any instant: `server_now_ms = local_monotonic_ms + median_offset_ms`.

2. **Target Playback Position Calculation:**
   - Host broadcasts `PlaybackState { track, is_playing, anchor_position_ms, anchor_server_ms, playback_rate, epoch }`.
   - Every listener computes desired position:
     `target_ms = anchor_position_ms + (is_playing ? (server_now_ms - anchor_server_ms) * playback_rate : 0)`.
   - Compute `drift_ms = local_position_ms - target_ms`.

3. **Three-Zone Drift Controller (`drift.rs`):**
   - **Deadband (`|drift_ms| <= 60ms`):** Do nothing — human auditory threshold for echo/offset in separate rooms is ~50–70ms; avoiding micro-adjustments keeps pitch rock-solid.
   - **Soft Tempo Nudge (`60ms < |drift_ms| <= 450ms`):** On `P1` Native PCM engine, smoothly nudge playback rate to `0.97x` (ahead) or `1.03x` (behind) using WSOLA pitch-preserving stretch until `|drift_ms| < 25ms`, then restore `1.00x`. (Zero audible stutter or seek click!)
   - **Hard Seek (`|drift_ms| > 450ms`):** Issue `PlaybackController::seek_ms(target_ms + estimated_seek_latency_ms)` with a 2-second cooldown to prevent seek oscillation loops.

### 1.2 Transport Resilience: WebSocket Primary + HTTP Long-Poll Fallback
Corporate firewalls and captive portals often block WebSocket upgrades:
1. Connect via `tokio-tungstenite` (`wss://<relay>/ws/rooms/<code_or_create>`).
2. Automatic exponential backoff reconnect (`1s -> 2s -> 4s -> 8s -> 15s` with jitter) carrying `participant_token` so reconnecting preserves host/member identity without duplicating participants.
3. If 2 consecutive WS handshakes fail with HTTP `400`/`403`/`426` or proxy reset, automatically fall back to `long_poll.rs` (`GET /http/rooms/<code>/events?since=<seq>` + `POST /http/rooms/<code>/command`) seamlessly!

### 1.3 Production Hardening of `crates/room-relay`
1. **Security & Abuse Controls:**
   - Cryptographically random 6-character crockford-base32 room codes (`2^30` space) + per-participant ephemeral bearer token (`32` random bytes, never logged).
   - Per-IP token-bucket rate limiter (`governor` or lightweight mutex map): max 5 room creates/min, max 20 commands/sec, max 32 participants per room, max 64 KB WS frame size.
   - Strict message validation: reject `local:*` track IDs at relay level (`400 LocalTracksUnsupported`), clamp reaction emoji/kind enum, bound queue length (`<= 200` tracks), bound display name (`<= 32` chars, stripped control chars).
   - Idle room garbage collector: sweep rooms with `0` active participants after 120 seconds (or total age `> 12 hours`).
2. **Automatic Host Failover:**
   - If the Host disconnects and does not reconnect within 15 seconds, promote the longest-connected remaining participant to Host, increment `epoch`, and broadcast `HostTransferred { new_host_id }`.
3. **Vote-to-Skip Consensus:**
   - Track per-track `skip_votes: HashSet<ParticipantId>` keyed by `(track_id, epoch)`. When `votes.len() * 2 > active_participants.len()` (strict majority), server authoritatively advances queue and resets vote set.

---

## 2. Concrete Implementation Plan (Step-by-Step)

### Step 1: Harden `crates/room-relay/`
- Add configurable env vars (`ROOM_RELAY_BIND`, `ROOM_RELAY_MAX_ROOMS`, `ROOM_RELAY_MAX_PARTICIPANTS`, `ROOM_RELAY_IDLE_TTL_SECS`).
- Implement `GET /healthz`, `WS /ws`, and `GET/POST /http/*` long-poll fallback endpoints using `axum` / `tokio-tungstenite`.
- Implement host failover timer, strict majority vote-skip, per-IP rate limiting, frame size cap, and structured JSON logging (with zero participant tokens logged).

### Step 2: Wire `crates/core/src/rooms/controller.rs` into `src/live_engine/handlers/rooms.rs`
- Read configured relay URL from `Settings` (defaulting to configurable `wss://...` or self-hosted `ws://127.0.0.1:8787` for local dev/testing; clearly show `"Configure Relay Server URL"` if unset instead of faking a room!).
- Bridge `RoomEvent` -> `PlaybackController`:
  - Incoming `PlaybackState` (when non-host) drives `play_track` / `pause` / `resume` / `drift_correct`.
  - Outgoing `PlaybackController` state changes (when host) broadcast `PlaybackState` with monotonic `epoch`.
  - When host plays a `local:*` track: block broadcast, set `rooms_local_track_warning = Some("Local files cannot be streamed to Listen Together peers")`, and keep remote room paused cleanly.
- Bridge `RoomSnapshot` -> `LiveUpdate::RoomUpdated` -> `src/store/rooms_slice.rs`.

### Step 3: Polish `src/screens/listen_together.rs`
- Add editable **Relay Server URL** field in room setup card (persisted in SQLite `kv_state`, validates `ws://` / `wss://`).
- Show real live connection indicator (`Connecting…`, `Connected (WS • 34ms RTT)`, `Connected (Long-Poll)`, `Reconnecting…`, or honest error message).
- Show real participants list with Host badge, RTT badge, Kick / Transfer Host controls (Host only, `>= 48dp`), Vote-to-Skip progress pill (`2 / 3 votes`), ephemeral floating reaction chips, and honest `"Share Room Code: {code}"` button.

---

## 3. Acceptance Criteria & Verification Tests (`tests/p5_rooms_relay_test.rs`)

1. **End-to-End 2-Client Sync Test:** Spin up `room-relay` on `127.0.0.1:0`; Host creates room, Guest joins via code -> Host plays track `t1` at `30_000ms` -> assert Guest receives `PlaybackState` and computes `target_ms` within `±35ms` of Host.
2. **Drift Controller Test:** Verify `drift = 40ms` -> `NoOp`; `drift = +200ms` -> `SoftRate(0.97)`; `drift = -700ms` -> `HardSeek`.
3. **Host Failover & Vote-Skip Test:** Drop Host WS connection -> advance clock 15s -> assert Guest becomes Host with incremented `epoch`. With 3 participants, verify 1 skip vote does not skip, 2nd skip vote authoritatively advances track.
4. **Long-Poll Fallback Test:** Block WS upgrade with HTTP `426` -> assert client automatically falls back to HTTP long-poll and still receives playback updates.
5. **Security & Abuse Test:** Assert oversized WS frame (`> 64 KB`) and `local:abc` track broadcast are rejected cleanly without crashing the relay.
