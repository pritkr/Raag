# ST-1 — Live `yt-dlp-ejs` Solver Bootstrap, Account/Premium Auto-Detection & Signed Remote Config Client

> **Wave:** Wave 1 (Short-Term P0) — Parallelizable with ST-2, ST-3, ST-4
> **Prerequisite (Wave 0):** Consumes **H0.0**'s canonical `AppPaths` (`crates/core/src/storage/paths.rs`) read-only. **Zero duplicate storage code!**
> **Parallelism Boundary:**
> - **Exclusive Write Scope:** `crates/core/src/solver/**`, `crates/core/src/innertube/remote_config.rs` (new), `crates/core/src/innertube/account_probe.rs` (new), `crates/core/src/storage/bootstrap.rs` (new ST-1 wiring helper), `assets/ejs/solver_manifest.json`, `tests/st1_persistence_ejs_remote_config_test.rs` (new).
> - **Targeted Additive Seam (only if Wave 0 didn't already expose):** `crates/core/src/innertube/mod.rs` (2 `pub mod` lines).
> - **Forbidden Edits:** `crates/core/src/storage/paths.rs` (owned by H0.0), `src/live_engine.rs`, `src/store/mod.rs`, `platform/**`, `crates/core/src/potoken/**`, `crates/core/src/playback/**`, `src/screens/**`. (Provide clean `bootstrap_st1_services(paths: &AppPaths, ...)` constructor in `crates/core/src/storage/bootstrap.rs` for Wave 1.5 stitch.)

---

## 1. Problem Statement & Deep Research Diagnosis

With **H0.0** resolving volatile `temp_dir()` storage into OS-standard `AppPaths`, ST-1 eliminates the remaining three P0 playback/protocol resilience blockers:

1. **Unwired EJS Solver Runtime:** `QuickJsSolver` (`crates/core/src/solver/mod.rs`) and `QuickJsRuntime` (`runtime.rs`) expect global functions `solveN(n)` and `solveSig(sig)`, whereas real `yt-dlp/ejs` releases (`yt.solver.lib.min.js` + `yt.solver.core.min.js`) expose a structured `jsc(input)` JSON contract (`{ type: "player", player: "<base.js>", requests: [{ type: "n" | "sig", challenges: [...] }] }`) using `meriyah` AST extraction. Furthermore:
   - `assets/ejs/solver_manifest.json` currently has `"sha256_hex": ""`.
   - `InnerTubeClient::set_player_solver()` is never called in live mode, and there is no YouTube `base.js` URL extractor/fetcher.
2. **Hardcoded `is_premium: false`:** `src/live_engine.rs:373` constructs `AuthSession` with `is_premium: false` and `account_name: None`, and `InnerTubeClient::set_premium_account()` is never invoked—so YouTube Music Premium subscribers never unlock 256 kbps Opus (`774`) / AAC (`141`) or the Premium GVS PO-Token bypass!
3. **Static `client_matrix.json`:** `InnerTubeClient::update_client_matrix()` exists, but startup never checks a signed remote config endpoint to absorb YouTube InnerTube protocol drift without requiring a full binary release. *(ST-1 owns the client-side Ed25519 verifier; LT-1 owns the automated 12h CI Ed25519 publisher using the exact same canonical envelope.)*

---

## 2. Architecture Overview

```text
┌────────────────────────────────────────────────────────────────────────────┐
│            App Boot (Background Tokio Runtime — Receives H0.0 `AppPaths`)  │
└───────────┬───────────────────────────────┬───────────────────────────────┘
            │                               │
            ▼                               ▼
┌───────────────────────────────────────┐   ┌───────────────────────────────────────────┐
│  LiveEjsBootstrapper (ST-1A)          │   │  RemoteConfigSyncService (ST-1C)          │
│  1. Fetch music.youtube.com/iframe_api│   │  • Fetches client_matrix.json + .sig      │
│     or watch page → extract player_url│   │  • Fetches solver_manifest.json + .sig    │
│     (/s/player/<hash>/base.js)        │   │  • Verifies Ed25519 pubkey + monotonic rev│
│  2. Cache raw base.js in              │   │  • Atomic temp→fsync→rename + ETag cache  │
│     `paths.cache_dir/js_solver/`      │   │  • Auto-rollback if schema/invariant fails│
│  3. Load verified yt-dlp-ejs bundle   │   └─────────────────────┬─────────────────────┘
│     (core + lib, SHA-256 checked)     │                         │
│  4. Preprocess player ONCE via jsc()  │                         ▼
│     → cache preprocessed_player on    │   ┌───────────────────────────────┐
│     disk + L1/L2 SQLite n-param memo  │   │  AccountProfileProbe (ST-1B)  │
│  5. Solve n / sig in <2ms on cache hit│   │  • Calls WEB_REMIX            │
└───────────────────────────────────────┘   │    account/account_menu       │
                                            │  • Extracts display name,     │
                                            │    channel handle, avatar,    │
                                            │    Premium badge/offer absence│
                                            │  • Persists into AuthSession  │
                                            │  • Calls set_premium_account()│
                                            └───────────────────────────────┘
```

---

## 3. Exact `crates/contract` Signatures (Frozen in Wave 0)

ST-1 consumes `AppPaths` (defined & resolved by **H0.0** in `crates/contract/src/storage.rs`) alongside these two additive structs in `crates/contract/src/storage.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountProfile {
    pub account_name: String,
    pub channel_handle: Option<String>,
    pub avatar_url: Option<String>,
    pub is_premium: bool,
    pub detected_at_unix_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteConfigStatus {
    pub matrix_revision: u64,
    pub matrix_source: String,
    pub solver_bundle_version: String,
    pub signature_verified: bool,
    pub last_sync_unix_secs: Option<u64>,
}
```

---

## 4. Deep-Dive Component Specifications

### 4.1 Integration with H0.0 `AppPaths` (`crates/core/src/storage/bootstrap.rs`)
- ST-1 does **not** re-implement OS path resolution or temp-dir migration (both are owned exclusively by **H0.0** in `crates/core/src/storage/paths.rs`).
- `bootstrap_st1_services(paths: &AppPaths, cache: Arc<SqliteCacheStore>, ...)` stores:
  - Raw & preprocessed YouTube player JS bundles under `paths.cache_dir.join("js_solver")`
  - Downloaded `yt-dlp-ejs` bundles under `paths.cache_dir.join("ejs")`
  - Verified remote `client_matrix.json` & `solver_manifest.json` snapshots under `paths.cache_dir.join("remote_config")`

### 4.2 ST-1A: Live YouTube `base.js` Extraction & `yt-dlp-ejs` `jsc` Protocol (`crates/core/src/solver/`)
Modern YouTube rotates `base.js` every 24–72 hours. Raw regex extraction breaks constantly; `yt-dlp/ejs` solves this via a two-stage AST pipeline (`meriyah` + `astring`) that preprocesses `base.js` once per `player_hash` and outputs a compact solver function.

- **New module `crates/core/src/solver/player_fetch.rs`:**
  - `PlayerScriptFetcher`:
    - Discovers the active player URL via `GET https://music.youtube.com/iframe_api` or `GET https://music.youtube.com/` (lightweight HTML regex matching `"/s/player/([0-9a-fA-F]{8,16})/[^"]*base\.js"`).
    - Caches `(player_hash, base_js_bytes)` on disk under `cache_dir/js_solver/raw_<player_hash>.js` with 6-hour revalidation TTL (and instant invalidation on HTTP 403).
    - Enforces strict host allowlist (`music.youtube.com`, `www.youtube.com`, `s.ytimg.com`) and 8 MB max body size cap.
- **Upgrade `crates/core/src/solver/ejs_protocol.rs`:**
  - Define typed `EjsRequest` / `EjsResponse` matching `yt-dlp/ejs`:
    ```rust
    #[derive(Serialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum EjsInput<'a> {
        Player {
            player: &'a str,
            requests: Vec<EjsChallengeRequest>,
            output_preprocessed: bool,
        },
        Preprocessed {
            preprocessed_player: &'a str,
            requests: Vec<EjsChallengeRequest>,
        },
    }

    #[derive(Serialize)]
    pub struct EjsChallengeRequest {
        #[serde(rename = "type")]
        pub kind: &'static str, // "n" or "sig"
        pub challenges: Vec<String>,
    }
    ```
  - **Two-Stage Caching for `< 2 ms` Steady-State Latency:**
    1. **Cold player hash:** Run `EjsInput::Player { player: base_js, requests, output_preprocessed: true }` through `yt.solver.lib.min.js` + `yt.solver.core.min.js` (`jsc(input)`). Persist the returned `preprocessed_player` JS string atomically to `cache_dir/js_solver/pre_<player_hash>.js` (10x–30x smaller and avoids re-running `meriyah` AST parsing on every track!).
    2. **Warm player hash:** Run `EjsInput::Preprocessed { preprocessed_player, requests }` — solves a new `n` or `sig` in ~15–40 ms in QuickJS-NG, then stores `(player_hash, raw_n) -> solved_n` in L1 RAM + L2 SQLite `EjsMemoCache` (`< 1 ms` on repeat hits).
  - **Backward Compatibility with Legacy Test Scripts:** In `QuickJsRuntime::eval_challenge`, detect whether the loaded bundle exposes `jsc` (official `yt-dlp-ejs` mode) OR `solveN`/`solveSig` (existing unit test harness mode) so all existing tests in `tests/core_unit_and_snapshots.rs` and `tests/architecture_gates_v2.rs` continue passing with zero breakage!
  - **QuickJS-NG Heap & Watchdog Tuning:** Because `meriyah` AST parsing of a 2.5 MB `base.js` peaks around 80–110 MB during the one-time preprocess pass, raise the one-time preprocess worker heap limit to `160 MB` (while keeping steady-state `Preprocessed` eval capped at `64 MB`) and drop/recreate the QuickJS `Runtime` immediately after preprocessing so RSS stays well under the `< 100 MB` steady-state budget!
  - **Desktop Deno Sidecar Preference:** On Desktop, if `RESONANCE_USE_DENO=1` or `deno` is detected on `PATH`, optionally prefer `DenoSidecarRuntime` for the one-time `Player` AST preprocess step (2–4x faster than QuickJS for AST string rebuilding) while still using embedded QuickJS-NG as zero-dependency default and Android primary.

### 4.3 ST-1B: Account Profile & YouTube Music Premium Auto-Detection (`crates/core/src/innertube/account_probe.rs`)
- Invoked asynchronously immediately after `sign_in()` and on cold boot when a saved `AuthSession` is loaded.
- **Endpoint:** `POST https://music.youtube.com/youtubei/v1/account/account_menu` via `InnerTubeClient::post_authenticated_web_remix("account/account_menu", json!({ "deviceTheme": "DEVICE_THEME_SUPPORTED" }))`.
- **Extraction Heuristics (Defensive Multi-Signal Parsing):**
  1. `accountName.runs[0].text` → `AccountProfile.account_name`.
  2. `channelHandle.runs[0].text` → `AccountProfile.channel_handle`.
  3. `accountPhoto.thumbnails.last().url` → `AccountProfile.avatar_url`.
  4. **Premium Detection:** Inspect `sections[].multiPageMenuSectionRenderer.items[]`:
     - Presence of `"Paid memberships"` / `"Manage membership"` (`SPunlimited` / `manage_membership` icon or endpoint) AND **absence** of `"Get Music Premium"` (`SPunlimited` upsell with `MUSIC_PREMIUM` offer) → `is_premium = true`.
     - **Secondary ground-truth confirmation during playback:** If any `WEB_REMIX` `/player` response returns audio itag `774` (256 kbps Opus) or `141` (256 kbps AAC), upgrade `is_premium` to `true` and persist!
- Updates `AuthSession` (`account_name`, `is_premium`), persists via `AuthStore::save_session`, calls `provider.set_premium_account(profile.is_premium)`, and emits `AppEvent::AuthStateChanged`.
- **Fallback Safety:** If `/account/account_menu` fails transiently (offline/5xx), preserve the last persisted `session.is_premium` and `session.account_name` rather than clobbering them to `false`.

### 4.4 ST-1C: Signed Remote `client_matrix.json` & `solver_manifest.json` Hot-Updater (`crates/core/src/innertube/remote_config.rs`)
- **Pinned Verification Public Key:** Embed a compile-time Ed25519 verifying key (`RESONANCE_CONFIG_ED25519_PUBKEY_HEX`, overridable in tests via constructor injection).
- **Signed Envelope Format:**
  - Remote endpoints (configurable via `RESONANCE_CONFIG_BASE_URL`, defaulting to project GitHub Pages / Release channel):
    - `GET <base>/client_matrix.json` + `GET <base>/client_matrix.json.sig` (raw 64-byte Ed25519 signature, hex/base64 encoded)
    - `GET <base>/solver_manifest.json` + `GET <base>/solver_manifest.json.sig`
  - **Verification Pipeline (Fail-Closed):**
    1. Enforce HTTPS (allow `http://127.0.0.1` only in `#[cfg(test)]` or explicit test builder).
    2. Cap response sizes (`client_matrix.json <= 128 KB`, `solver_manifest.json <= 32 KB`, `yt.solver.*.js <= 4 MB`).
    3. Verify Ed25519 signature over exact raw response bytes BEFORE JSON parsing.
    4. Validate schema version (`schema_version == 1`), non-empty enabled clients, and **anti-rollback monotonic check** (`remote.updated_at_unix_secs >= current.updated_at_unix_secs`).
    5. Validate `AuthMode` invariants: **hard-reject any remote matrix that attempts to set `ANDROID_VR` or `IOS` to `cookies_or_anonymous`!** (Prevents supply-chain downgrade of cookie isolation!)
    6. Write atomically (`tmp` → `sync_all` → `rename`) into `cache_dir/remote_config/`.
    7. If `solver_manifest.json` specifies a newer `bundle_version` with a non-empty `sha256_hex`, download `updatable_remote_url`, verify exact SHA-256 against the signed manifest's `sha256_hex`, and call `solver.install_ejs_bundle_update(...)`.
- **ETag / Conditional Refresh:** Store `ETag` and `last_checked_unix_secs` in SQLite `metadata_cache`; skip network fetch if checked within 6 hours (unless forced on playback circuit-breaker exhaustion).
- **Startup Behavior:** Load last verified cached config synchronously from disk (0 ms network wait), then spawn background refresh task so cold start stays `< 20 ms`.

---

## 5. Security, Privacy & Concurrency Edge Cases

1. **Supply-Chain Tampering on Remote Matrix:** Even if the CDN is compromised, missing/invalid Ed25519 signature OR any attempt to give `ANDROID_VR`/`IOS` cookie permissions is rejected before touching memory or disk.
2. **Untrusted JS in `base.js`:** QuickJS-NG runs with zero filesystem/network globals, strict memory cap, and 10s interrupt watchdog on its dedicated `resonance-qjs` thread.
3. **Single-Flight Player Fetch:** Concurrent track resolutions on startup share a `tokio::sync::Mutex` / `OnceCell` per `player_hash` so Resonance never downloads `base.js` twice simultaneously.
4. **Disk Exhaustion Protection:** Cap `cache_dir/js_solver/` to the 3 most recent `player_hash` bundles (evicting older `raw_*.js` and `pre_*.js` files automatically).
5. **Zero Secret Leaks:** `AccountProfileProbe` never logs raw response cookies or `SAPISID`; `redact_secrets()` wraps all error strings.

---

## 6. Offline Verification Test Suite (`tests/st1_persistence_ejs_remote_config_test.rs`)

All tests run 100% offline using `tempfile::TempDir`, ephemeral `ed25519_dalek::SigningKey`, and local `127.0.0.1:0` `hyper` mock servers:

1. `bootstrap_st1_places_ejs_and_remote_config_under_app_paths_cache_dir` — passes an H0.0 `AppPaths` fixture into `bootstrap_st1_services` and verifies `js_solver/`, `ejs/`, and `remote_config/` live exclusively inside `paths.cache_dir` (never polluting `data_dir`).
2. `ejs_jsc_protocol_preprocesses_player_once_and_memoizes_n_and_sig` — serves mock `base.js` + mock `jsc(input)` solver script; verifies cold preprocess writes `pre_<hash>.js`, second solve uses `Preprocessed` path, and third identical `n` hits SQLite memo with `js_eval_count` unchanged and `< 5 ms` latency.
3. `ejs_solver_evicts_old_player_bundles_keeping_latest_three` — verifies disk cleanup cap.
4. `account_probe_detects_premium_and_free_accounts_and_preserves_on_network_error` — serves mock `/account/account_menu` payloads for Premium and Free users; verifies `is_premium` toggles `InnerTubeClient::account_tier()` (`PremiumSignedIn` vs `FreeSignedIn`) and transient `500` preserves prior persisted tier.
5. `remote_config_accepts_valid_ed25519_signature_and_rejects_tampered_or_rollback` — signs a new `client_matrix.json` with test Ed25519 key -> accepted & persisted; tampers 1 byte -> rejected; older timestamp -> rejected; `ANDROID_VR` with `cookies_or_anonymous` (even validly signed!) -> rejected by hard security invariant!
6. `remote_solver_manifest_downloads_and_verifies_sha256_before_hot_swap` — verifies SHA-256 mismatch aborts update and keeps previous bundle active.

```bash
cargo test --test st1_persistence_ejs_remote_config_test -- --nocapture
cargo test --test core_unit_and_snapshots --test architecture_gates_v2
```
