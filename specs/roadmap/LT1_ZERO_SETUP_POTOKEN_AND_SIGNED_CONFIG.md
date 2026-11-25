# LT-1 — Zero-Setup Embedded PO-Token Attestation & Automated Ed25519 Signed Config Publisher

> **Wave:** Wave 2 (Long-Term P1) — Parallelizable with LT-2 and LT-3 (requires ST-1 & ST-4 completed in Wave 1)
> **Parallelism Boundary:**
> - **Exclusive Write Scope:** `crates/core/src/potoken/**`, `src/bin/client_probe.rs`, `src/bin/sign_config.rs` (new CLI signer for CI), `.github/workflows/probe.yml`, `crates/core/src/innertube/matrix.rs` (canary health metadata fields only), `tests/lt1_potoken_attestation_and_signed_publisher_test.rs` (new).
> - **Forbidden Edits:** `crates/core/src/casting/**`, `src/updater/**`, `nsh.json`, `.github/workflows/release.yml`, `platform/android/**` (declare the JNI attestation contract trait here; Android Kotlin WebView execution delegates through ST-4's `ResonanceNativeBridge` in Wave 3 stitch).

---

## 1. Deep Research: 2025–2026 YouTube BotGuard PO-Token Landscape

### Why Free & Anonymous Users Still Break Without Zero-Setup PO Tokens
In YouTube's InnerTube architecture:
- **`WEB_REMIX` (Signed-in Premium):** Bypasses GVS PO-Token requirement (`PoTokenPolicy::RequiredForWebFreeBypassedByPremium`), needing only `yt-dlp-ejs` (`n`/`sig`).
- **`WEB_REMIX` (Signed-in Free or Anonymous):** Requires a **GVS Proof-of-Origin (`poToken`)** bound to `dataSyncId` / `visitorData` (passed in both `serviceIntegrityDimensions.poToken` and `&pot=` on `googlevideo.com` stream URLs) AND increasingly a **Player PO Token** bound to `video_id`. Without it, `googlevideo.com` serves HTTP `403` or falls back to degraded/low-bitrate clients.
- **Current Limitation:** Resonance's `live_engine.rs` currently only supports `RESONANCE_POTOKEN_URL` (requiring users to self-host `bgutil-ytdlp-pot-provider`) or `RESONANCE_POTOKEN_VM_SCRIPT` (operator-supplied JS). **End users should never have to run a Docker container or Node sidecar to play music!**

### The 5-Tier Zero-Setup PO-Token Chain
We upgrade `crates/core/src/potoken/` into a self-healing, multi-tier provider chain where **zero user configuration works out of the box** while power users can still plug in external sidecars:

```text
                        ┌──────────────────────────────────────────────┐
                        │   CachingPoTokenProvider (Single-Flight +    │
                        │   Proactive 80% TTL Background Refresh +     │
                        │   Instant 403 Invalidation + SQLite Persist) │
                        └──────────────────────┬───────────────────────┘
                                               │
                                               ▼
                        ┌──────────────────────────────────────────────┐
                        │        ChainedPoTokenProvider (Priority)     │
                        ├──────────────────────────────────────────────┤
                        │ Tier 1: BgutilHttpPoTokenProvider            │
                        │         (if RESONANCE_POTOKEN_URL is set —   │
                        │          strict 127.0.0.1/::1 loopback only) │
                        ├──────────────────────────────────────────────┤
                        │ Tier 2: RustyPipeBotguardProvider (Desktop)  │
                        │         (if `rustypipe-botguard` binary is   │
                        │          found in PATH or app_data/bin/)     │
                        ├──────────────────────────────────────────────┤
                        │ Tier 3: EmbeddedWebViewAttestationProvider   │
                        │         (ZERO-SETUP DEFAULT!                 │
                        │          • Desktop: Hidden NSH WebView       │
                        │            (WKWebView/WebView2/WebKitGTK)    │
                        │          • Android: Hidden Android WebView   │
                        │            via JNI ResonanceNativeBridge     │
                        │          Runs real BotGuard challenge from   │
                        │          /youtubei/v1/att/get in real DOM)   │
                        ├──────────────────────────────────────────────┤
                        │ Tier 4: QuickJsVmAttestationBridge           │
                        │         (if RESONANCE_POTOKEN_VM_SCRIPT set) │
                        ├──────────────────────────────────────────────┤
                        │ Tier 5: DegradedColdStartFallback            │
                        │         (marks status=DegradedColdStart so   │
                        │          ClientMatrix routes cleanly to      │
                        │          ANDROID_VR / VISIONOS without stall)│
                        └──────────────────────────────────────────────┘
```

---

## 2. Deep-Dive Architecture Specifications

### 2.1 Embedded Zero-Setup BotGuard WebView Attestation (`crates/core/src/potoken/webview_attestation.rs`)
Why a hidden platform WebView beats pure QuickJS for BotGuard:
- Google's BotGuard VM obfuscates real DOM/Canvas/Navigator/WebCrypto fingerprints; pure headless JS engines (like QuickJS without DOM) are frequently flagged or require massive `jsdom` + `canvas` shims (which is why `rustypipe-botguard` has to bundle Deno + JSDOM).
- Both **Desktop** (via NSH's native `WKWebView` / `WebView2` / `WebKitGTK`, already permitted in `nsh.json`) and **Android** (built-in Chromium `WebView`) ALREADY ship a genuine, zero-overhead browser engine on the device!

**Exact Attestation Protocol Flow (`AttestationOrchestrator`):**
1. **Challenge Acquisition (Rust — `reqwest` + `rustls`):**
   - `POST https://music.youtube.com/youtubei/v1/att/get?prettyPrint=false` (using `WEB_REMIX` context + `visitorData` or signed-in `SAPISIDHASH` if free signed-in).
   - Parse `bgChallenge`: `{ interpreterUrl, program, globalName, challenge }`.
   - Fetch the BotGuard interpreter JS from `interpreterUrl` (strictly validate host `*.gstatic.com` / `www.google.com` / `www.youtube.com` and `<= 2 MB` cap).
2. **Challenge Execution (Ephemeral Hidden WebView Bridge):**
   - Trait `WebViewBotguardRunner: Send + Sync`:
     - Loads a minimal sandboxed `about:blank` / `https://music.youtube.com/robots.txt` origin HTML harness in a `1×1` hidden ephemeral WebView (zero network access except BotGuard init if needed, 6s hard timeout, destroyed immediately after minting to keep RSS `< 100 MB`!).
     - Evaluates the `interpreterJs` + `program` to produce the `botguardResponse` string and a reusable `WebPoMinter` snapshot token.
3. **Integrity Token Exchange (Rust — `reqwest`):**
   - `POST https://www.google.com/js/th/...` or `https://jnn-pa.googleapis.com/$rpc/google.internal.waa.v1.Waa/GenerateIT` with `[requestKey, botguardResponse]` → receives `{ integrityToken, estimatedTtlSecs, mintRefreshThreshold }`.
4. **Content-Bound PO-Token Minting (`Gvs` / `Player` / `Subs`):**
   - Uses the minted `integrityToken` (valid ~6–12 hours!) to derive content-bound tokens (`visitorData` for anonymous `Gvs`, `dataSyncId` for signed-in free `Gvs`, `video_id` for `Player`/`Subs`) locally without re-running the WebView on every track!
   - **Cache Persistence:** Store the **encrypted/obfuscated `integrityToken` + `expires_at` + `identity_key`** in SQLite (`potoken_cache` table, never logged) so cold app restarts within the 6h TTL need **zero WebView launches**!

### 2.2 `RustyPipeBotguardProvider` (`crates/core/src/potoken/rustypipe_bg.rs`)
- Optional Tier 2 provider on Desktop (`#[cfg(not(target_os = "android"))]`).
- Discovers `rustypipe-botguard` binary from `RESONANCE_BOTGUARD_BIN`, `app_paths.data_dir/bin/rustypipe-botguard`, or system `PATH`.
- Spawns with bounded 8s timeout, `stdin(null)`, JSON stdout parsing, and strict argument validation (`video_id` / `visitor_data` regex-checked against `[A-Za-z0-9_=%-]{1,256}` to prevent CLI flag injection!).
- Automatically reports `PoTokenDiagnostics { active_provider: "rustypipe-botguard", status: Healthy, .. }`.

### 2.3 Proactive 80% TTL Background Refresh & Single-Flight Hardening (`crates/core/src/potoken/cache.rs`)
- Upgrade `CachingPoTokenProvider`:
  - **Single-flight per identity key:** Concurrent requests for `(scope, identity_key)` share a single in-flight `tokio::sync::watch` future (zero thundering herd when a playlist starts + 50% prefetch triggers).
  - **Proactive Refresh at 80% TTL:** When a cached token is served and `remaining_ttl <= 20% * original_ttl`, return the still-valid token immediately (0 ms playback stall!) while spawning a background refresh task to swap in a fresh token before expiry.
  - **Negative-Cache Backoff:** If all tiers fail, cache the failure for 30s (exponential up to 5m, reset on network/auth change) so every track click doesn't stall for 6s waiting on a broken attestation endpoint before falling back to `ANDROID_VR` / `VISIONOS`!
  - **403 Coalesced Invalidation:** `LoopbackProxy` HTTP `403` calls `invalidate(PoTokenScope::Gvs)` with a 10s debounce so rapid parallel chunk retries only trigger one fresh mint.

### 2.4 Automated 12h Signed Config Publisher (`src/bin/client_probe.rs`, `src/bin/sign_config.rs`, `.github/workflows/probe.yml`)
Transform `.github/workflows/probe.yml` from an artifact-only diagnostic job into a **zero-downtime automated remote config delivery pipeline** feeding ST-1's `RemoteConfigSyncService`:

```text
  GitHub Actions Cron (Every 12h)
    │
    ├─ 1. Run `client_probe` against live probe videos (free, premium itag, age-restricted)
    │     with cookies OFF (and optional CI canary cookie secret if configured)
    ├─ 2. Fetch latest `yt-dlp/ejs` release metadata + compute exact SHA-256 of `yt.solver.lib.min.js`
    │     & `yt.solver.core.min.js`, smoke-test `jsc()` inside embedded `rquickjs` on live `base.js`!
    ├─ 3. Safety Canary Gate (`client_probe --verify-canary`):
    │     • Abort publish if 0 clients pass OR if `ANDROID_VR`/`IOS` auth_mode != `anonymous_only`
    │     • Abort publish if `yt-dlp-ejs` fails QuickJS smoke test
    │     • Bump monotonic `updated_at_unix_secs`
    ├─ 4. Sign artifacts with Ed25519 CI secret (`RESONANCE_CONFIG_ED25519_PRIVKEY_HEX`):
    │     `sign_config --key-env RESONANCE_CONFIG_ED25519_PRIVKEY_HEX target/dist/client_matrix.json`
    │     `sign_config --key-env RESONANCE_CONFIG_ED25519_PRIVKEY_HEX target/dist/solver_manifest.json`
    │     → outputs `client_matrix.json.sig` and `solver_manifest.json.sig`
    └─ 5. Publish atomically to `gh-pages` branch (`/config/v1/`) with `Cache-Control` + rollback history
          (clients using ST-1 pick up new matrix & verified EJS SHA-256 within 6h, zero app rebuild!)
```

- **`src/bin/sign_config.rs`:** Small deterministic binary using `ed25519-dalek` that supports `--sign`, `--verify`, and `--keygen` subcommands; fails non-zero if a matrix violates `AuthMode::AnonymousOnly` invariants before signing!
- **Canary Rollback Safety:** Every publish archives `history/<timestamp>/client_matrix.json` and writes `last_known_good.json`; if a subsequent probe detects regression, CI opens/updates a GitHub Issue (`label: protocol-drift`) automatically without pushing a broken matrix!

---

## 3. Security, Privacy & Concurrency Edge Cases

1. **Zero Cookie/PO-Token Contamination:** `ANDROID_VR` and `IOS` remain strictly hardcoded to skip `PoTokenProvider` and `serviceIntegrityDimensions` (already enforced in `InnerTubeClient::resolve_with_client` + re-asserted in `sign_config` pre-sign gate).
2. **Host Allowlist in Attestation Fetcher:** `interpreterUrl` from `/att/get` MUST be validated against HTTPS + `gstatic.com` / `google.com` / `youtube.com` suffixes before downloading; reject arbitrary third-party URLs.
3. **Key Compromise Mitigation:** Keep the Ed25519 config-signing key completely separate from the NSH app-binary release-signing key (LT-2). Embed two public keys in binary (`PRIMARY_PUBKEY` + `BACKUP_PUBKEY`) to support seamless key rotation without stranding older app builds.
4. **Secret Hygiene:** `PoToken::fmt(Debug)` masks value as `[REDACTED_POT]`; `AttestationOrchestrator` never logs `integrityToken`, `botguardResponse`, or `dataSyncId`.
5. **WebView Memory Hygiene:** Destroy the ephemeral attestation WebView immediately after `GenerateIT` completes; verify RSS returns to `< 100 MB`.

---

## 4. Offline Verification Test Suite (`tests/lt1_potoken_attestation_and_signed_publisher_test.rs`)

1. `chained_potoken_provider_falls_through_tiers_and_respects_negative_cache_backoff` — configures Tier 1 (failing bgutil), Tier 2 (failing botguard), Tier 3 (mock `WebViewBotguardRunner`); asserts Tier 3 mints token, diagnostics report `embedded-webview`, and when all tiers fail, negative cache prevents repeated 6s stalls within backoff window.
2. `caching_provider_coalesces_concurrent_mints_and_proactively_refreshes_at_80pct_ttl` — launches 12 concurrent `mint_token` calls (asserts 1 underlying call); advances mock clock past 80% TTL -> serves cached token immediately while triggering background refresh.
3. `webview_attestation_rejects_untrusted_interpreter_hosts` — feeds `/att/get` response pointing `interpreterUrl` to `https://evil.example/bg.js`; asserts hard rejection before HTTP GET.
4. `rustypipe_botguard_rejects_shell_metacharacters_in_binding` — validates strict binding sanitizer before subprocess spawn.
5. `sign_config_and_probe_canary_enforce_ed25519_and_anonymous_isolation` — generates ephemeral Ed25519 keypair, verifies `sign_config` signs valid `client_matrix.json` & `solver_manifest.json`, rejects invalid signatures, and refuses to sign any matrix that enables cookies on `ANDROID_VR` or `IOS`.

```bash
cargo test --test lt1_potoken_attestation_and_signed_publisher_test -- --nocapture
cargo run --bin client_probe -- --offline-sim --output target/test_matrix.json
```
