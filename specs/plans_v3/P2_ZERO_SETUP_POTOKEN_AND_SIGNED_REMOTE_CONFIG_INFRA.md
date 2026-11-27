# P2: Zero-Setup PO Token Attestation & Signed Remote Config Infrastructure

> **Wave:** A (Parallel)
> **Status of Existing Code:** `crates/core/src/potoken/*` (`attestation.rs`, `bgutil.rs`, `cache.rs`, `chain.rs`, `rustypipe_bg.rs`, `webview_attestation.rs`), `crates/core/src/innertube/remote_config.rs`, `src/bin/sign_config.rs`, `.github/workflows/probe.yml`, and `config/client_matrix.json` exist, **but**:
> 1. `remote_config.rs` still points at placeholder signing public key / unhosted URL or falls back to embedded config without live signed updates.
> 2. Android WebView BotGuard JS attestation (`webview_attestation.rs`) lacks the Kotlin bridge (`BotGuardAttestationBridge.kt`) actually executing Google's BotGuard `Waa` / `jnn` challenge in an offscreen WebView.
> 3. `InnerTubeClient` fallback circuit breaker does not yet trigger automatic PO Token refresh or distinguish `403` expired-token errors from `403` missing-PO-token vs cipher-mismatch errors.
> **Exclusive Owned Paths:** `crates/core/src/potoken/*`, `crates/core/src/innertube/remote_config.rs`, `crates/core/src/innertube/matrix.rs`, `src/bin/sign_config.rs`, `.github/workflows/probe.yml`, `config/client_matrix.json`, `platform/android/src/main/kotlin/dev/resonance/ytmusic/BotGuardAttestationBridge.kt`, `src/live_engine/handlers/potoken_config.rs`, `tests/p2_potoken_config_test.rs`.

---

## 1. Deep Technical Research & Architecture

### 1.1 How YouTube Proof-of-Origin (PO) Tokens Actually Work (2025–2026)
YouTube enforces **BotGuard (`jnn-pa.googleapis.com` / `youtubei/v1/att/get`)** proof-of-origin tokens across `WEB`, `WEB_REMIX`, `MWEB`, `IOS`, and increasingly `TVHTML5` clients:
- **GVS PO Token (`&pot=<token>` appended to `googlevideo.com/videoplayback` URL):** Proves the playback client executed BotGuard attestation; bound to either a visitor data ID (`visitorData` for anonymous sessions) or a session/content binding (`videoId` or `DATASYNC_ID` for signed-in sessions). Without it, `WEB`/`WEB_REMIX` streams return HTTP `403 Forbidden` or degrade to low-bitrate formats.
- **Player PO Token (`serviceIntegrityDimensions.poToken` inside `/youtubei/v1/player` body):** Required for `WEB_REMIX` / `MWEB` player requests to return full adaptive Opus 251 / AAC 140/141 formats instead of `LOGIN_REQUIRED` / `UNPLAYABLE`.
- **Token TTL:** Session/visitor-bound GVS tokens stay valid for **6–12 hours** (cacheable in memory + SQLite `transient_cache` keyed by `(client_name, binding_hash)`).

### 1.2 Zero-Setup Multi-Tier PO Token Provider Chain (`crates/core/src/potoken/chain.rs`)

Users should **NEVER** have to install Docker, Node.js, or Python just to play music. Implement a strict ordered fallback chain with automatic health scoring:

```
PoTokenChain::acquire(binding: PoTokenBinding, client: ClientKind)
  │
  ├─ Tier 0: SQLite/RAM Cache (`potoken/cache.rs`)
  │    └─ Valid if `expires_at > now + 300s` & `binding_hash` matches.
  │
  ├─ Tier 1 (Android): Offscreen Native WebView BotGuard Attestation
  │    └─ `BotGuardAttestationBridge.kt` loads minimal `https://music.youtube.com/sw.js_data` /
  │       BotGuard VM script in a sandboxed, cookie-isolated WebView (no visible UI),
  │       calls `window.bgChallenge.execute(identifier)` -> returns `(poToken, ttl)`.
  │
  ├─ Tier 2 (Desktop & Android Fallback): Pure-Rust / Embedded QuickJS BotGuard Minting
  │    └─ Calls `/youtubei/v1/att/get` -> obtains `bgChallenge` + `interpreterUrl` ->
  │       evaluates interpreter inside sandboxed `rquickjs` runtime (with minimal DOM/canvas/
  │       navigator polyfill shim in `crates/core/src/potoken/rustypipe_bg.rs`) ->
  │       POSTs attestation to `/v1/GenerateIT` -> mints `poToken` for `visitorData`/`videoId`.
  │
  ├─ Tier 3 (Optional Power-User Sidecar): Local `bgutil-ytdlp-pot-provider` HTTP (`127.0.0.1:4416`)
  │    └─ Enabled ONLY if user explicitly configures a localhost URL (`127.0.0.1` / `[::1]` enforced!).
  │       Never allow non-loopback HTTP URLs for security.
  │
  └─ Tier 4 (Graceful Matrix Degradation): Client Fallback without PO Token
       └─ `ANDROID_VR` (anonymous, never sends cookies!) -> `TV_EMBEDDED` -> `IOS` (anonymous).
          Update `PoTokenDiagnostics` honestly to `DegradedColdStart` or `Unconfigured`.
```

### 1.3 Signed Hot-Updatable `client_matrix.json` & `yt-dlp-ejs` Bundle Pipeline
YouTube rotates client versions, `cver` strings, and player JS cipher transforms every few weeks. Instead of forcing app binary updates on every breakage:

1. **Signed Manifest Envelope (`SignedRemoteBundle`):**
   ```json
   {
     "schema_version": 2,
     "revision": 42,
     "published_at_unix": 1764000000,
     "min_app_version": "0.1.0",
     "matrix_sha256": "...",
     "ejs_bundle_sha256": "...",
     "ejs_bundle_version": "2026.11.20",
     "payload_b64": "...",
     "ed25519_sig_b64": "..."
   }
   ```
2. **Cryptographic Verification (`ed25519-dalek`):**
   - Pin the release Ed25519 public key bytes in `crates/core/src/innertube/remote_config.rs` (plus an optional `RESONANCE_CONFIG_PUBKEY_HEX` override for self-hosters/CI tests).
   - **Rollback & Anti-Downgrade Protection:** Reject any remote bundle whose `revision < persisted_revision` or whose signature fails `VerifyingKey::verify_strict`.
   - **Canonical Signing:** Sign `b"resonance-remote-config-v2\n" || canonical_payload_bytes` so signature confusion across bundle types is impossible.
3. **Automated GitHub Actions Probe & Auto-Publish (`.github/workflows/probe.yml`):**
   - Runs every 6 hours via `cron` + manual `workflow_dispatch`.
   - Executes `cargo run --bin client_probe -- --json` against live YouTube Music.
   - Verifies client priority matrix & latest `yt-dlp-ejs` solver bundle pass stream resolution + HTTP 206 byte-range probe.
   - Signs updated `client_matrix.json` + `solver_manifest.json` using `src/bin/sign_config.rs` with repo secret `RESONANCE_CONFIG_SIGNING_KEY` and publishes to the `remote-config` branch / GitHub Pages endpoint.

---

## 2. Concrete Implementation Plan (Step-by-Step)

### Step 1: Android Offscreen BotGuard Attestation (`BotGuardAttestationBridge.kt` + `webview_attestation.rs`)
- Create `platform/android/src/main/kotlin/dev/resonance/ytmusic/BotGuardAttestationBridge.kt`:
  - Uses a single lazy offscreen `WebView` on Android's main Looper with `javaScriptEnabled = true`, `allowFileAccess = false`, `allowContentAccess = false`, and an allowlist `WebViewClient` permitting ONLY `https://*.youtube.com`, `https://*.google.com`, and `https://*.gstatic.com` / `https://jnn-pa.googleapis.com`.
  - Exposes JNI entrypoint `nativeOnPoTokenResult(requestId: Long, token: String?, ttlSecs: Long, error: String?)`.
  - Runs with a strict 12-second timeout and clears WebView memory cache on failure.
- Wire `crates/core/src/potoken/webview_attestation.rs` to await a `tokio::sync::oneshot` keyed by `requestId`.

### Step 2: Desktop Embedded QuickJS BotGuard Minting (`rustypipe_bg.rs` + `attestation.rs`)
- Complete `attestation.rs` to:
  1. POST `https://music.youtube.com/youtubei/v1/att/get` with client context + `engagementType: "ENGAGEMENT_TYPE_UNBOUND"` (or `PLAYBACK`).
  2. Parse `bgChallenge.interpreterUrl`, `program`, `globalName`, and `challenge` token.
  3. Cache the downloaded `interpreterUrl` JS file in SQLite `transient_cache` (TTL 24h, SHA-256 keyed).
  4. Execute inside a dedicated sandboxed `rquickjs::Context` (`memory_limit = 32 MB`, `max_stack_size = 512 KB`, timeout guard 5s) with deterministic minimal `window`/`document`/`navigator`/`performance.now`/`atob`/`btoa` shims.
  5. POST the resulting BotGuard snapshot to `https://jnn-pa.googleapis.com/$rpc/google.internal.waa.v1.Waa/GenerateIT` and mint the content/GVS `poToken`.
- Never log raw `poToken`, `visitorData`, or `bgResponse` payloads (log only `provider_name`, `ttl_secs`, and first 6 chars of SHA-256 hash).

### Step 3: Strict Error Taxonomy & Self-Healing Circuit Breaker (`crates/core/src/innertube/matrix.rs`)
- Classify stream failures into 4 distinct causes:
  1. `ExpiredSignedUrl` (`403` after `expire` timestamp) -> refresh `/player` URL with same client.
  2. `PoTokenRejected` (`403` on `WEB_REMIX`/`IOS` before expiry) -> invalidate cached PO token, mint fresh token once, else demote client in circuit breaker for 15 minutes and fail over to `ANDROID_VR` (cookie-stripped!).
  3. `CipherSolverMismatch` (`n`-param throttle or missing signature transform) -> invalidate cached `player.js`, fetch fresh player script + trigger remote EJS bundle check.
  4. `AccountAuthRequired` (`LOGIN_REQUIRED` / age-restricted) -> route exclusively to authenticated `WEB_REMIX` / `TVHTML5` (NEVER leak cookies to `ANDROID_VR`/`IOS`!).

### Step 4: Signed Remote Config & Hot EJS Reload (`remote_config.rs` + `sign_config.rs`)
- Upgrade `sign_config.rs` to sign both `client_matrix.json` and `ejs_bundle.js` into a single `SignedRemoteBundle` with domain-separated Ed25519 signature.
- On app startup and `"Sync Config"` button press in Settings (`src/live_engine/handlers/potoken_config.rs`):
  1. Fetch signed bundle with `If-None-Match` ETag header (10s timeout, max 2 MB body cap).
  2. Verify Ed25519 signature & anti-downgrade `revision >= local_revision`.
  3. Atomically swap `ClientMatrix` and call `QuickJsSolver::install_ejs_bundle(...)` without restarting playback!
  4. Push honest `RemoteConfigStatus` (`signature_verified: true/false`, `matrix_revision`, `solver_bundle_version`) to `settings.rs`.

---

## 3. Security Invariants & Acceptance Tests (`tests/p2_potoken_config_test.rs`)

1. **Ed25519 Tamper Rejection:** Mutating 1 byte of `client_matrix.json` or `ejs_bundle` or downgrading `revision` MUST return `RemoteConfigError::InvalidSignature` / `DowngradeRejected` and keep the last known good bundle intact.
2. **Loopback-Only `bgutil` Enforcement:** Passing `http://192.168.1.50:4416` or `https://evil.example` to `BgUtilProvider` MUST be rejected at validation time; only `127.0.0.1` and `[::1]` are allowed.
3. **Cookie Isolation Under Failover:** When `WEB_REMIX` fails with `PoTokenRejected` and matrix falls back to `ANDROID_VR` / `IOS`, assert via wiremock that zero `Cookie` or `Authorization: SAPISIDHASH` headers are transmitted.
4. **Provider Chain Fallback Order:** Mock Tier 1 failure -> verify Tier 2 executes and caches token in SQLite -> verify second call hits Tier 0 cache with zero network calls.
5. **Log Redaction Test:** Verify `PoTokenDiagnostics::detail` and tracing spans never contain raw token strings.
