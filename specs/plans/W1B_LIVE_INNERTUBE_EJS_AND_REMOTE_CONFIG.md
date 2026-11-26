# W1B — Live Two-Stage `yt-dlp-ejs` Solver, Premium Tier Auto-Detection & Fail-Closed Signed Remote Config

> **Wave:** Wave 1 (Parallel Agent 1B — Runs concurrently with W1A, W1C, W1D, W1E)
> **Exclusive Write Scope:**
> - `crates/core/src/solver/{mod.rs,manifest.rs,memo.rs,runtime.rs,base_js.rs,ejs_bundle.rs}`
> - `crates/core/src/innertube/{remote_config.rs,account_probe.rs}` (new)
> - `assets/ejs/solver_manifest.json`, `config/client_matrix.json`
> - `tests/w1b_ejs_premium_remote_config_test.rs` (new)
> **Read-Only Dependencies (From Wave 0):** `AppPaths` (`crates/core/src/storage/paths.rs`), `ed25519-dalek` (`Cargo.toml`), `rquickjs 0.12.0`.
> **Stitch Hooks (For Wave 1.5 Integration Agent):**
> - `RemoteConfigBootstrap::load_and_refresh(paths, http)` called in `src/live_engine.rs` startup
> - `EjsSolverService::attach_to_client(&client)` wired in `src/live_engine.rs`
> - `AccountProfileProbe::probe_and_apply(&client, session)` invoked after auth load / sign-in
> **Forbidden Edits:** `Cargo.toml`, `crates/contract/**`, `src/live_engine.rs`, `src/store/**`, `src/screens/**`, `platform/**`, `crates/core/src/potoken/**`.

---

## 1. Ground-Truth Problems & Research Context

1. **Unwired Challenge Solver:** `InnerTubeClient::player_solver_js` starts as `None` and is never populated. `assets/ejs/solver_manifest.json` has `"sha256_hex": ""` and no bundled solver script or `base.js` fetcher. Every ciphered `WEB_REMIX` stream immediately fails or suffers YouTube's ~50 KB/s `n`-parameter throttling.
2. **Hardcoded Non-Premium Status:** `is_premium_account` is hardcoded `false` in `live_engine.rs:437` and `client.rs:105`, preventing YouTube Music Premium subscribers from receiving `itag 141` (256 kbps AAC) / `itag 774` (256 kbps Opus) and causing unnecessary GVS PO-token fallbacks.
3. **Static Unverified `client_matrix.json`:** YouTube rotates InnerTube client versions (`WEB_REMIX`, `ANDROID_VR`, `IOS`, `TVHTML5`) frequently. Without a cryptographically signed remote config updater, every API shift requires shipping a full binary release.

### Deep Research Finding: Why Two-Stage `yt-dlp-ejs` in QuickJS-NG Beats Regex Scraping
YouTube's `base.js` (~2–3 MB heavily minified ECMAScript) rotates obfuscation patterns (control-flow flattening, array-rotation string tables, prototype traps) weekly. Modern `yt-dlp` solved this with **EJS (External JavaScript)** using AST analysis (`meriyah` + `astring` bundled inside `yt-dlp-ejs` `jsc` bundle):
- **Stage 1 — Preprocess (Once per `player_hash`, cached to SQLite):** Feed `base.js` + `yt-dlp-ejs` AST bundle into QuickJS-NG (`rquickjs 0.12.0`). The AST extractor locates the exact `n`-transform and `sig`-decipher functions and emits a compact, pure, standalone JS solver module (~10–30 KB). Cache `(player_hash -> preprocessed_solver_js)` in SQLite `metadata_cache` (TTL 14 days).
- **Stage 2 — Execute (Per-track, `< 2 ms`, memoized):** Load the small preprocessed solver into a warm, sandboxed QuickJS context and evaluate batch `{ n: [...], sig: [...] }` challenges. Memoize `(player_hash, challenge_kind, input) -> output` in `SolverMemoCache` (LRU 4,096 entries) so replays, seeks, and range requests cost ~0 µs.

---

## 2. Target Architecture

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│  App Startup / Background Refresh (Tokio Runtime — Never Blocks UI Thread)   │
│                                                                              │
│  [1] RemoteConfigManager (Fail-Closed Ed25519 Verifier)                      │
│      ├─ Fetch `client_matrix.json` + `.sig` (ETag conditional)               │
│      ├─ Fetch `solver_manifest.json` + `.sig` + EJS bundle (SHA-256)         │
│      ├─ Verify Ed25519 signature against compiled-in pinned public key       │
│      ├─ Enforce anti-rollback (`published_at >= cached.published_at`)        │
│      └─ Enforce security invariant: ANDROID_VR / IOS == AnonymousOnly!       │
│                                                                              │
│  [2] BaseJsFetcher + TwoStageEjsSolver (`rquickjs 0.12.0` Sandbox)           │
│      ├─ Discover `player_url` (`/s/player/<hash>/.../base.js`)               │
│      ├─ Cache raw `base.js` under `paths.cache_dir/ejs/<hash>.base.js`       │
│      ├─ Stage 1 (Once per hash): AST preprocess → `preprocessed.<hash>.js`   │
│      │   (64 MB heap cap, 10s interrupt watchdog, zero network/FS globals)   │
│      └─ Stage 2 (Per stream): Batch evaluate `n` + `sig` + LRU memoize       │
│                                                                              │
│  [3] AccountProfileProbe                                                     │
│      ├─ Authenticated `account/account_menu` + `browse(FEmusic_home)`        │
│      ├─ Extract: account_name, avatar_url, channel_handle, is_premium        │
│      └─ Call `client.set_premium_account(is_premium)` + update AuthSession   │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Detailed Implementation Specification

### 3.1 `BaseJsFetcher` (`crates/core/src/solver/base_js.rs`)
1. **Player URL Discovery (Multi-Source Cascade):**
   - Primary: Parse `jsUrl` (`"/s/player/<8-hex>/player_ias.vflset/en_US/base.js"`) from the latest `music.youtube.com` HTML or cached `https://www.youtube.com/iframe_api` (~3 KB lightweight response!).
   - Validate strict allowlist before fetching: `https://www.youtube.com/s/player/...` or `https://music.youtube.com/s/player/...` (reject any non-YouTube host to eliminate SSRF).
   - Extract `player_hash` (strict alphanumeric regex `^[a-fA-F0-9]{8,16}$`) for cache keying.
2. **Single-Flight Disk + Memory Cache:**
   - Store under `paths.cache_dir/ejs/base_<player_hash>.js` (atomic write via `.tmp` + rename).
   - Coalesce concurrent cold-start requests for the same `player_hash` using `tokio::sync::OnceCell` / `Mutex<HashMap<String, Arc<Notify>>>`.
   - Keep only the 3 most recent `base_*.js` files on disk (LRU prune older player versions to cap cache at ~9 MB).

### 3.2 Two-Stage `EjsSolverService` (`crates/core/src/solver/ejs_bundle.rs` & `runtime.rs`)
1. **Manifest Verification (`assets/ejs/solver_manifest.json` + `manifest.rs`):**
   - Populate canonical schema:
     ```json
     {
       "schema_version": 2,
       "bundle_version": "0.3.2",
       "published_at_unix_secs": 1764000000,
       "entrypoint_fn": "jsc",
       "sha256_hex": "<real-64-hex-sha256-of-bundled-or-remote-ejs>",
       "download_url": "https://raw.githubusercontent.com/<org>/resonance-config/main/ejs/yt.solver.bundle.min.js",
       "min_core_version": "0.1.0"
     }
     ```
   - Never execute any EJS script whose computed SHA-256 does not match the Ed25519-verified `solver_manifest.json`.
   - Support an embedded fallback/minimal solver stub for offline tests while loading the verified remote/cached `yt-dlp-ejs` bundle in production.
2. **Sandboxed QuickJS-NG Execution (`runtime.rs`):**
   - Dedicated `tokio::task::spawn_blocking` worker (NEVER on UI thread or Tokio async worker threads).
   - Hard resource ceilings on `rquickjs::Runtime`:
     - `rt.set_memory_limit(64 * 1024 * 1024)` (64 MB)
     - `rt.set_max_stack_size(1024 * 1024)` (1 MB)
     - `rt.set_interrupt_handler(...)` aborting execution after `10_000 ms` (Stage 1) or `2_000 ms` (Stage 2).
   - Strip dangerous globals (no `fetch`, `require`, `fs`, `XMLHttpRequest`, `eval` bridge to host).
   - **Stage 1 Contract (`preprocess_player`):**
     - Passes `{ type: "player", player: base_js_str, requests: [] }` → receives `{ preprocessed_player: String }`.
     - Persists `preprocessed_player` in SQLite `metadata_cache` under key `ejs:preprocessed:{bundle_version}:{player_hash}` (TTL 14 days).
   - **Stage 2 Contract (`solve_challenges`):**
     - Checks `SolverMemoCache` first for every `(player_hash, kind, input)`.
     - Only sends cache misses in a single batched QuickJS invocation:
       `{ type: "preprocessed", preprocessed_player, requests: [{ type: "n", challenges: [...] }, { type: "sig", challenges: [...] }] }`.
     - Validates every returned value is non-empty, ASCII URL-safe, and differs from known error sentinels; logs redacted timings (`stage2_ms`, `cache_hits`, `cache_misses`).
     - If QuickJS panics, hits OOM, or times out, drop and recreate the runtime cleanly and return `CoreError::Solver(...)` so `ClientMatrix` automatically falls back to the next client (`ANDROID_VR` / `IOS`).

### 3.3 Premium Account Probe (`crates/core/src/innertube/account_probe.rs`)

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountProfile {
    pub account_name: Option<String>,
    pub channel_handle: Option<String>,
    pub avatar_url: Option<String>,
    pub is_premium: bool,
    pub detected_via: PremiumDetectionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PremiumDetectionSource {
    AccountMenuBadge,
    HighBitrateFormatProbe,
    AnonymousDefault,
}
```

**Detection Strategy (Layered & Non-Destructive):**
1. **Primary (`account/account_menu` on `WEB_REMIX`):**
   - Inspect response JSON for active Music Premium indicators:
     - `activeAccountHeaderRenderer` / `multiPageMenuSectionRenderer` items containing `paid_memberships` / `"Paid memberships"` vs `"Get Music Premium"` upsell CTA (absence of `"Get Music Premium"` CTA + presence of `MUSIC_PREMIUM` / `UNLIMITED` badge or `"Paid memberships"` item confirms Premium).
   - Extract real `account_name`, `channel_handle`, and `avatar_url` to populate the top-right avatar and Settings Account Card!
2. **Secondary Confirmation (Passive Stream Format Observation):**
   - Whenever any authenticated `player` call returns `itag 141` (256 kbps AAC) or `itag 774` (256 kbps Opus) in `adaptiveFormats`, automatically promote `is_premium = true` on the active session and persist to `AuthStore`.
3. **Cache TTL:** Cache `AccountProfile` for 6 hours in memory/SQLite; immediately re-probe on sign-in, sign-out, or account switch.

### 3.4 Fail-Closed Ed25519 Signed Remote Config (`crates/core/src/innertube/remote_config.rs`)

**Envelope & Verification Rules:**
- Every remote artifact (`client_matrix.json`, `solver_manifest.json`) has a companion raw 64-byte (or hex/base64-encoded) Ed25519 signature file (`.sig`) signed over the exact raw UTF-8 bytes of the JSON file.
- **Compiled-In Pinned Verifying Key:** `const CONFIG_VERIFYING_KEY_HEX: &str = "...";` (overridable ONLY in `#[cfg(test)]` via `RemoteConfigVerifier::with_public_key_for_test(...)` — NEVER overridable via env var in release builds!).
- **Strict Fail-Closed Verification Pipeline:**
  1. Verify Ed25519 signature (`ed25519_dalek::VerifyingKey::verify_strict`) over exact downloaded bytes.
  2. Parse JSON into strongly typed `ClientMatrixConfig` / `SolverManifest`.
  3. **Anti-Rollback Check:** Require `remote.published_at_unix_secs >= current_cached.published_at_unix_secs`. Reject downgraded/replayed old configs!
  4. **Security Invariant Audit (Mandatory for `client_matrix.json`):**
     - Assert every entry with `client_name` in `["ANDROID_VR", "IOS"]` has `auth_mode == AuthMode::AnonymousOnly`.
     - Assert `clients` list is non-empty and every `api_url` uses `https://` on `*.youtube.com` / `*.googleapis.com`.
     - If ANY invariant fails, reject the remote config even if validly signed, log a redacted security warning, and retain the last-known-good cached or bundled config.
  5. **Atomic Swap:** Write verified bytes + `.sig` to `paths.cache_dir/remote_config/*.tmp` → `fs::rename`, persist ETag for `If-None-Match` 304 bandwidth savings.

---

## 4. Deterministic Offline Verification (`tests/w1b_ejs_premium_remote_config_test.rs`)

1. `ed25519_verifier_accepts_valid_config_and_rejects_tampered_or_rolled_back` — generates ephemeral Ed25519 keypair, tests valid apply, 1-bit tamper rejection, rollback timestamp rejection, and `304 Not Modified` ETag preservation against local `127.0.0.1:0` HTTP server.
2. `ed25519_verifier_rejects_config_weakening_android_vr_auth_isolation` — signs a malicious `client_matrix.json` setting `ANDROID_VR` to `Authenticated`; asserts verifier **rejects** it and preserves bundled fallback.
3. `two_stage_ejs_solver_preprocesses_caches_and_memoizes_n_and_sig` — serves mock `base.js` + mock `jsc` bundle from local HTTP server, verifies Stage 1 runs once per `player_hash` (SQLite hit on restart), Stage 2 transforms `n` and `sig` accurately, and repeat calls hit `SolverMemoCache` with 0 QuickJS invocations.
4. `quickjs_sandbox_enforces_timeout_and_memory_cap_and_recovers` — runs infinite loop and memory-bomb JS payloads; asserts watchdog interrupts cleanly within budget and subsequent valid solve succeeds.
5. `account_profile_probe_detects_premium_and_free_accounts_accurately` — replays fixture `account_menu` JSON for Free vs. Premium accounts + passive `itag 141` promotion, asserting `is_premium` and `account_name` update accurately.

```bash
cargo test --test w1b_ejs_premium_remote_config_test -- --nocapture
cargo test --test core_unit_and_snapshots
```
