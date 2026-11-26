# W2D — Real Multi-User "Listen Together" Rooms: Clock Sync, Hysteresis Drift Control & Self-Hostable Relay

> **Wave:** Wave 2 (Parallel Agent 2D — Runs concurrently with W2A, W2B, W2C after Wave 1.5 Stitch)
> **Exclusive Write Scope:**
> - `crates/core/src/rooms/{mod.rs,protocol.rs,clock_sync.rs,drift.rs,client.rs}` (new)
> - `crates/room-relay/{Cargo.toml,src/main.rs,src/state.rs,src/ws.rs,src/long_poll.rs}` (new standalone workspace binary crate — NEVER linked into the Resonance client!)
> - `src/store/rooms_slice.rs` (new)
> - `src/screens/listen_together.rs` (new)
> - `tests/w2d_listen_together_rooms_test.rs` (new)
> **Read-Only Dependencies:** `crates/contract/src/rooms.rs`, `tokio-tungstenite = "0.26"` (`rustls-tls-webpki-roots`), `rubato` slew hook (W2A).
> **Stitch Hooks (For Wave 2.5 Integration Agent):**
> - Register `crates/room-relay` in root `Cargo.toml` workspace members (optional/standalone)
> - Upgrade `"Radio Mix"` tab back to `"Listen Together"` (with two sub-tabs: `Rooms` + `Radio Mix`) in `src/screens/shell.rs`
> - Wire `RoomClient` playback commands/telemetry & W2A `set_slew_ratio` hook in `src/live_engine.rs` & `src/store/mod.rs`
> **Forbidden Edits:** `Cargo.toml`, `crates/contract/**`, `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/{shell,settings,home,library,now_playing,queue,lyrics,explore,search}.rs`, `platform/**`.

---

## 1. Ground-Truth Problem Fixed

Previously, `src/screens/settings.rs` dedicated a top-level nav tab to `"Listen Together — Shared Radio Sessions"`, which had zero networking, rooms, or synchronization—just a local `autoplay_radio` toggle. Wave 1A honestly renamed that view to `"Radio Mix"`. Wave 2D builds **true, low-latency, audiophile-grade multi-user synchronized listening**.

---

## 2. Deep Technical Architecture: Why Naive WebSocket Sync Sounds Awful (and How We Solve It)

Naive "Listen Together" implementations send `{ position_ms: 42000 }` every 2 seconds and force clients to call `seek()`. That fails catastrophically because:
1. **Uncompensated Network & Clock Skew:** Two phones' wall clocks easily differ by 200–1,500 ms, and mobile LTE jitter adds ±150 ms.
2. **Audible Seek Stutters:** Repeated hard seeks cause clicks, codec buffer flushes, and endless ping-pong loops.
3. **Firewalls:** Corporate/school Wi-Fi networks frequently block WebSocket upgrades.

### Resonance's Host-Authoritative Sync Engine

```text
┌────────────────────────────────────────────────────────────────────────────┐
│  Standalone Relay (`crates/room-relay` on `axum 0.8` — Stateless/In-Mem)   │
│  • Zero audio proxying! Relays ONLY tiny JSON control frames & NTP pings   │
│  • Room Code `XXXX-XXXX` (Crockford Base32, 40-bit entropy, no 0/O/1/I)    │
│  • Primary Transport: WebSocket `/v1/rooms/:code/ws`                       │
│  • Automatic Fallback: HTTP Long-Poll `/v1/rooms/:code/poll?seq=N`         │
│  • Per-IP rate limiting, max 32 peers/room, 4h idle room TTL               │
└──────────────────────────▲──────────────────────────▲──────────────────────┘
                           │ WS / Long-Poll (rustls)  │
        ┌──────────────────┴─────────┐    ┌───────────┴──────────────────┐
        │  HOST Client               │    │  LISTENER Client(s)          │
        │  • Owns `host_secret`      │    │  • Runs Cristian/NTP Clock   │
        │    (redacted in all logs!) │    │    Offset estimator θ        │
        │  • Publishes monotonic     │    │  • Resolves `video_id` via   │
        │    `state_rev: u64` +      │    │    OWN YTM account/client    │
        │    `anchor_server_ms` +    │    │    (ZERO cookie sharing!)    │
        │    `anchor_position_ms` +  │    │  • 3-Zone Hysteresis Drift   │
        │    `playing: bool`         │    │    Controller:               │
        │  • Handles Queue/VoteSkip  │    │    <60ms: deadband (1.000x)  │
        │    based on RoomPermissions│    │    60–450ms: smooth slew     │
        └────────────────────────────┘    │    >=450ms: hard seek        │
                                          └──────────────────────────────┘
```

---

## 3. Core Algorithms & Protocol Specification

### 3.1 Cristian/NTP 4-Timestamp Median-RTT Clock Sync (`crates/core/src/rooms/clock_sync.rs`)
- On join (burst of 5 pings spaced 200 ms apart) and every 15 seconds thereafter, client sends `Ping { t1_local_ms }`; relay responds immediately with `Pong { t1_local_ms, t2_server_rx_ms, t3_server_tx_ms }`; client stamps `t4_local_ms`:
  $$\text{RTT} = (t_4 - t_1) - (t_3 - t_2), \qquad \theta = \frac{(t_2 - t_1) + (t_3 - t_4)}{2}$$
- Maintain a sliding window of the last 8 samples; discard outliers with `RTT > 2 * min_rtt` and compute the **median offset $\theta$** of the 5 lowest-RTT samples.
- **Target Expected Playback Position at Any Local Time $t_{\text{local}}$:**
  ```rust
  pub fn expected_position_ms(&self, anchor: &PlaybackAnchor, now_local_ms: i64) -> u64 {
      if !anchor.is_playing {
          return anchor.position_ms;
      }
      let now_server_ms = now_local_ms + self.offset_ms;
      let elapsed_ms = (now_server_ms - anchor.server_timestamp_ms).max(0) as u64;
      anchor.position_ms.saturating_add(elapsed_ms)
  }
  ```

### 3.2 3-Zone Hysteresis Drift Controller (`crates/core/src/rooms/drift.rs`)
Evaluated every 500 ms against `drift_ms = local_position_ms as i64 - expected_position_ms as i64`:

| Zone | Condition | Action | Why |
| :--- | :--- | :--- | :--- |
| **Zone 1: Deadband** | `\|drift_ms\| < 60` (or exiting slew once `\|drift_ms\| < 20`) | `DriftAction::Nominal` (`slew_ratio = 1.0`) | Hysteresis prevents rapid oscillation between speed-up and slow-down |
| **Zone 2: Micro-Slew** | `60 <= \|drift_ms\| < 450` sustained for **3 consecutive ticks** (1.5s) AND native PCM engine active (W2A) | `DriftAction::Slew(ratio)` where `ratio = (1.0 - drift_ms / 15_000.0).clamp(0.985, 1.015)` | Converges 300 ms drift cleanly over ~20 seconds at ≤1.5% rate change—completely inaudible, zero clicks! |
| **Zone 3: Hard Seek** | `\|drift_ms\| >= 450` (or `>= 250` when slew unavailable) with **3-second post-seek cooldown** | `DriftAction::HardSeek(expected_position_ms + seek_lead_ms)` | Instantly recovers from tab sleep, buffering stall, or track start without seek-storming |

- **Zero-Hour Track Start Scheduling:** When Host switches tracks, Host broadcasts `anchor_server_ms = server_now + 600 ms`, `position_ms = 0`. Every peer immediately resolves & pre-buffers the track and begins playback simultaneously at `server_now == anchor_server_ms`!

### 3.3 Transport Resilience & Security (`client.rs` & `crates/room-relay/`)
1. **Transport Failover:** Attempt WebSocket first (`tokio-tungstenite` over `rustls`, 5s handshake timeout). If blocked (HTTP 400/403/502 or TCP reset on upgrade), seamlessly switch to HTTP Long-Poll (`GET /v1/rooms/:code/poll?after_rev=N`, 25s server hold) + `POST /v1/rooms/:code/command` with identical JSON frame types!
2. **Reconnect & Host Handoff:** Exponential backoff (`500ms → 1s → 2s → 4s → 8s` + jitter). If Host drops for > 30s, relay promotes the longest-connected listener (or closes room cleanly if empty).
3. **Permissions Model (`RoomPermissions`):**
   - `HostOnly`: Only host controls playback & queue; listeners can send emoji reactions & suggest tracks.
   - `OpenQueue`: Any participant can append/reorder Up Next; host retains play/pause/seek authority.
   - `VoteSkip`: Listeners can cast a 1-per-track skip vote; auto-advances when `votes > participants / 2`.
4. **Strict Privacy & Security Invariants:**
   - **Zero Credential Sharing:** Only public YouTube `video_id` + track title/artist/duration + playback anchor are transmitted. Local files (`local:*`) cannot be broadcast to remote rooms (clear UI hint: `"Local files are private and cannot be shared in rooms"`).
   - `host_secret` (256-bit random token) is held only in host memory, passed via `X-Host-Secret` header / auth frame, and stripped by `redact_secrets()`.
   - Relay enforces max frame size (16 KB), max 32 participants/room, and per-connection token-bucket rate limit (10 msgs/sec).

### 3.4 UI & UX Design (`src/screens/listen_together.rs` & `src/store/rooms_slice.rs`)
- **Two Clean Entry Actions:** `"Create Room"` (one tap → generates `XXXX-XXXX` invite code + `"Copy Invite Code"` + Relay URL configurable in Settings for self-hosters) and `"Join Room"` (formatted `XXXX-XXXX` input + display name).
- **Active Room View:**
  - Compact Room Header: Room Code badge + Copy button + Connection status dot (`Synced · ±12 ms` / `Reconnecting...` / `Long-Poll`) + Participant avatar stack + `"Leave Room"`.
  - Shared Now Playing + Shared Queue + Quick Reaction bar (ephemeral floating micro-reactions honoring `reduced_motion`).
  - Host Controls drawer: Permissions selector (`Host Only` / `Open Queue` / `Vote Skip`) + Kick/Transfer Host menu.

---

## 4. Deterministic Offline Verification (`tests/w2d_listen_together_rooms_test.rs`)

1. `clock_sync_estimator_rejects_jitter_spikes_and_converges_offset` — feeds 8 synthetic NTP exchanges with 2 artificially delayed 900 ms cellular jitter spikes; asserts median offset $\theta$ is accurate within `±5 ms`.
2. `three_zone_hysteresis_controller_respects_deadband_slew_and_seek_cooldown` — steps through `30 ms` (nominal), `180 ms` × 3 ticks (slew `~0.988`), `15 ms` (exits slew cleanly), and `800 ms` (hard seek once, then cooldown suppresses repeat seek).
3. `room_relay_websocket_and_long_poll_end_to_end_sync` — boots in-process `room-relay` on `127.0.0.1:0`, connects Host via WS and Listener via Long-Poll fallback, tests room creation, join, track change broadcast, vote-skip threshold, unauthorized host command rejection (`403`), and local-file broadcast rejection.
4. `listen_together_screen_renders_create_join_and_active_room_headless` — headless NSH UI test verifying empty lobby, join code validation, and active room participant list with zero raw colors.

```bash
cargo test --test w2d_listen_together_rooms_test -- --nocapture
```
