# W3A — 5-Tier Zero-Setup PO-Token Chain & 12-Hour Automated Signed Remote Config CI

> **Wave:** Wave 3 (Parallel Agent 3A — Runs concurrently with W3B, W3C after Wave 2.5 Stitch)
> **Exclusive Write Scope:**
> - `crates/core/src/potoken/{mod.rs,attestation.rs,bgutil.rs,cache.rs,chain.rs,webview_bg.rs,rustypipe_bg.rs}`
> - `src/bin/client_probe.rs`
> - `.github/workflows/probe.yml`
> - `tests/w3a_potoken_chain_and_signed_probe_test.rs` (new)
> **Read-Only Dependencies:** `AppPaths` (W0), `RemoteConfigVerifier` + Ed25519 envelope format (W1B), `rustypipe` / `rustypipe-botguard` (optional feature), NSH offscreen WebView / Android JNI WebView hooks (W1E/W2C).
> **Stitch Hooks (For Wave 3.5 Integration Agent):**
> - Construct `ChainedPoTokenProvider::zero_setup_default(paths, webview_bridge)` in `src/live_engine.rs`
> - Surface concise diagnostic status (`Active / Cached / Fallback`) ONLY inside **Settings → Advanced & Diagnostics**
> **Forbidden Edits:** `Cargo.toml`, `crates/contract/**`, `src/live_engine.rs`, `src/store/**`, `src/screens/**`, `crates/core/src/innertube/remote_config.rs`, `platform/**`.

---

## 1. Ground-Truth Problem & Research Context

YouTube increasingly requires cryptographically attested **Proof-of-Origin (PO) Tokens** minted via Google's **BotGuard / Web Anti-Abuse (WAA)** VM for `WEB_REMIX` and GVS (`googlevideo.com`) media URLs—otherwise returning `HTTP 403` or `"Sign in to confirm you're not a bot"`.

Today, Resonance's `potoken` chain requires power users to manually configure `RESONANCE_POTOKEN_URL` (`bgutil`) or `RESONANCE_POTOKEN_VM_SCRIPT`. Average users get zero-setup degradation and fragile anonymous fallbacks.

**Goal of W3A:** Make PO-Token minting **100% zero-setup out of the box** on Desktop and Android while preserving strict `ANDROID_VR`/`IOS` anonymous isolation, AND automate 12-hour live InnerTube probing that automatically publishes Ed25519-signed `client_matrix.json` and `solver_manifest.json` updates consumed fail-closed by W1B.

---

## 2. 5-Tier Zero-Setup PO-Token Chain Architecture

```text
┌────────────────────────────────────────────────────────────────────────────┐
│  `ChainedPoTokenProvider` (Single-Flight Coalesced + 80% TTL Refresh)      │
│                                                                            │
│  [Fast Path] `PoTokenCache` (In-Memory LRU + SQLite Encrypted-Safe Cache)  │
│   • Keyed by `(scope: SessionGvs | ContentBound(video_id), binding_id)`    │
│   • Hit at < 80% TTL → Instant return (~5 µs)                              │
│   • Hit at 80%–100% TTL → Instant return + background proactive refresh!   │
│                                                                            │
│  [Miss Path — Single-Flight Coalescer per binding_id (no thundering herd)] │
│   ├─ Tier 1: `WebViewBotGuardProvider` (Zero-Setup Native Engine)          │
│   │   • Hidden 0×0 offscreen WebView on Desktop (NSH) / Android (JNI)      │
│   │   • Loads minimal local `about:blank` BotGuard harness executing       │
│   │     Google WAA challenge with real browser engine fingerprint          │
│   │   • Caches VM integrity token (valid ~6–12h) to mint per-video tokens  │
│   │     in < 15 ms with zero network roundtrips!                           │
│   ├─ Tier 2: `RustyPipeBotGuardProvider` (Pure-Rust Snapshot Runtime)      │
│   │   • Headless / WebView-unavailable fallback using `rustypipe-botguard` │
│   │   • Persists VM snapshot under `paths.cache_dir/potoken/bg_snap.bin`   │
│   ├─ Tier 3: `BgUtilHttpProvider` (Optional Local Sidecar `127.0.0.1:4416`)│
│   │   • Auto-discovered ONLY when `RESONANCE_POTOKEN_URL` is configured    │
│   ├─ Tier 4: `VmScriptProvider` (`RESONANCE_POTOKEN_VM_SCRIPT`)            │
│   └─ Tier 5: `ColdStartFallback`                                           │
│       • Signals `ClientMatrix` to step cleanly to anonymous `ANDROID_VR` / │
│         `IOS` client (strictly WITHOUT browser cookies or `&pot=`!)        │
└────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Deep Technical Design Details

### 3.1 Strict Binding, TTL & Protocol Isolation Invariants
1. **Two Token Scopes:**
   - **Session / GVS PO-Token** (bound to `visitor_data` for anonymous web, or authenticated `SAPISID` account identifier for signed-in web): attached to `serviceIntegrityDimensions.poToken` AND `&pot=` on `googlevideo.com` URLs for `WEB_REMIX`.
   - **Content-Bound PO-Token** (bound to `video_id`): minted locally in `< 15 ms` from the cached BotGuard integrity token (`mint(video_id)`) without re-running the full attestation network handshake!
2. **Proactive 80% TTL Refresh (`cache.rs`):**
   - Typical BotGuard integrity token TTL is 6–12 hours (`21_600s – 43_200s`).
   - When `elapsed >= 0.80 * ttl`, serve the still-valid token immediately (zero playback latency!) and spawn ONE background refresh task guarded by `in_flight_refresh: Mutex<HashSet<String>>`.
3. **Hard Auth Isolation Invariant (Non-Negotiable):**
   - `ChainedPoTokenProvider::resolve_for_client(client_profile, binding)` MUST return `Ok(None)` immediately if `client_profile.auth_mode == AuthMode::AnonymousOnly` (`ANDROID_VR` / `IOS`) — **never** attach Web GVS PO-tokens to anonymous mobile/VR clients!
4. **Secret Hygiene:** Every PO-token, integrity token, and BotGuard payload is redacted via `redact_secrets()` before logging or error reporting.

### 3.2 Single-Flight Minting Coalescer (`chain.rs`)
- When a user opens Home or rapidly skips 5 tracks on cold start, 5 concurrent tasks may request a session PO-token simultaneously.
- Use `HashMap<String, tokio::sync::broadcast::Sender<Result<PoToken, String>>>` keyed by `binding_key` so **only 1 attestation runs** and all 5 waiters receive the exact minted token simultaneously.
- Per-tier circuit breaker: if Tier 1 WebView times out (`> 8s`) or crashes, trip Tier 1 for 10 minutes and fall straight through to Tier 2 (`rustypipe-botguard`) with zero delay.

### 3.3 Automated 12-Hour Probe & Ed25519 Signed Config CI (`src/bin/client_probe.rs` & `.github/workflows/probe.yml`)
1. **`client_probe` Enhancements (`src/bin/client_probe.rs`):**
   - Add `--offline-sim` mode (runs deterministic probe checks against local fixture server for unit/CI testing with zero internet).
   - Add `--emit-signed-dir <DIR>` + `--signing-key-env RESONANCE_CONFIG_ED25519_PRIVKEY`:
     - Probes every client profile in `config/client_matrix.json` (`WEB_REMIX`, `ANDROID_VR`, `IOS`, `TVHTML5`) against a known royalty-free public audio track, measuring resolve latency, HTTP status, and playable bitrate.
     - Re-ranks healthy clients, bumps `published_at_unix_secs` monotonically, verifies the **hard invariant** (`ANDROID_VR` & `IOS` MUST remain `AuthMode::AnonymousOnly`), verifies latest `yt-dlp-ejs` bundle SHA-256 into `solver_manifest.json`, and signs both files with Ed25519 (`*.json.sig`).
     - Self-verifies its own generated `.sig` files using W1B's `RemoteConfigVerifier` before exiting `0`!
2. **`.github/workflows/probe.yml`:**
   - Schedule: `cron: '0 */12 * * *'` + `workflow_dispatch`.
   - Fails closed: if fewer than 2 clients pass live stream probe, aborts publication and opens/updates a GitHub Alert issue instead of publishing a broken matrix!
   - Publishes verified `client_matrix.json`, `client_matrix.json.sig`, `solver_manifest.json`, `solver_manifest.json.sig` to the dedicated `config-dist` branch / GitHub Pages endpoint consumed by W1B.

---

## 4. Deterministic Offline Verification (`tests/w3a_potoken_chain_and_signed_probe_test.rs`)

1. `chained_potoken_cascades_across_tiers_and_coalesces_concurrent_callers` — spawns 10 concurrent requests while Tier 1 fails and Tier 2 succeeds after 50 ms delay; asserts Tier 2 executes **exactly once** (single-flight) and all 10 callers get the token.
2. `proactive_80pct_ttl_refresh_returns_immediately_and_updates_in_background` — seeds cache at 85% TTL with mock clock; asserts immediate return of old token + asynchronous replacement with fresh token.
3. `anonymous_only_clients_never_receive_web_potokens` — calls `resolve_for_client` for `ANDROID_VR` and `IOS`; asserts `Ok(None)` even when valid Web PO-tokens are cached.
4. `client_probe_offline_sim_emits_and_self_verifies_ed25519_signed_artifacts` — runs `client_probe` signing pipeline with an ephemeral test Ed25519 keypair in `TempDir`, then validates outputs with W1B `RemoteConfigVerifier`.
5. `potoken_redaction_strips_all_token_material_from_errors_and_logs` — verifies `redact_secrets` scrubs `pot=`, `poToken`, and raw base64 tokens.

```bash
cargo test --test w3a_potoken_chain_and_signed_probe_test -- --nocapture
cargo test --test potoken_mutations_test
```
