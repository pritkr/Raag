# SPEC A — Dependency-Injected `PoTokenProvider` & Policy-Gated `&pot=` Attachment

> Parallelism boundary: owns `crates/contract/src/potoken.rs`, `crates/core/src/potoken/` (new module), and targeted policy/URL hooks in `crates/core/src/innertube/{matrix.rs,client.rs}`. Zero edits to `platform/desktop.rs`, `cache/db.rs`, or `innertube/mutations.rs`.

## 1. Exact `crates/contract` Signatures

### `crates/contract/src/potoken.rs` (new, re-exported in `lib.rs`)
```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PoTokenScope {
    /// GVS (Google Video Server) token bound to `visitorData` (anonymous) or session `dataSyncId` (free signed-in).
    Gvs,
    /// Player / playback-minted token bound to `video_id`.
    Player,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoTokenRequest {
    pub client_name: String,
    pub scope: PoTokenScope,
    pub video_id: String,
    pub visitor_data: Option<String>,
    pub signed_in: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoToken {
    pub value: String,
    pub scope: PoTokenScope,
    pub expires_at_unix_secs: u64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PoTokenError {
    #[error("po token provider unavailable: {0}")]
    Unavailable(String),
    #[error("po token mint timeout after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
    #[error("po token mint failed: {0}")]
    MintFailed(String),
    #[error("po token forbidden for client {client_name} ({reason})")]
    PolicyForbidden { client_name: String, reason: String },
}

#[async_trait]
pub trait PoTokenProvider: Send + Sync {
    async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError>;
    fn invalidate(&self, scope: PoTokenScope);
    fn provider_name(&self) -> &'static str;
}
```
### `crates/contract/src/error.rs` (additive variant on `CoreError`)
```rust
    #[error("po token error: {0}")]
    PoToken(#[from] PoTokenError),
```

## 2. File-by-File Implementation Plan

- **`crates/core/src/innertube/matrix.rs`**:
  - Add `#[serde(rename = "required")] Required` variant to `PoTokenPolicy` alongside `RequiredForWebFreeBypassedByPremium`, `RequiredIosGuard`, `NotRequired`, `Optional`.
  - Add pure helper on `PoTokenPolicy`:
    ```rust
    pub fn requires_gvs_po_token(self, tier: AccountTier) -> bool {
        match self {
            Self::Required => true,
            Self::RequiredForWebFreeBypassedByPremium => !tier.is_premium(),
            Self::NotRequired | Self::Optional | Self::RequiredIosGuard => false,
        }
    }
    ```
  - Update `ClientMatrixEntry::is_eligible`: `PoTokenPolicy::Required => has_gvs_po_token`.
  - Add `ClientMatrix::entry_for(&self, kind: ClientKind) -> Option<&ClientMatrixEntry>`.
- **`crates/core/src/potoken/mod.rs` (new, exported in `crates/core/src/lib.rs`)**:
  - `NoopPoTokenProvider`: default when no provider is configured (`Err(PoTokenError::Unavailable("not configured"))`).
  - `StaticPoTokenProvider`: wraps `RwLock<Option<PoToken>>` (replaces ad-hoc `set_gvs_po_token` path cleanly while preserving backward compatibility).
  - `SidecarPoTokenProvider`: calls configurable localhost HTTP sidecar (`http://127.0.0.1:<port>/get_pot`, strict loopback-only host check, 4s timeout, bounded response <= 4 KB) with `{ "content_binding": video_id_or_visitor_data }`.
  - `CachingPoTokenProvider<P: PoTokenProvider>`:
    - Caches `Gvs` tokens by `(client_name, visitor_data, signed_in)` with 60s expiry skew margin and single-flight deduplication via `tokio::sync::Mutex<()>` (prevents thundering herd on concurrent track resolves).
    - `invalidate(PoTokenScope::Gvs)` clears cached token on upstream HTTP 403.
  - `pub fn attach_pot_param(stream_url: &str, po_token: &str) -> CoreResult<String>`:
    - Parses via `url::Url`, rejects non-HTTPS/non-loopback URLs, appends/replaces `pot=<token>` only if absent/stale, preserves existing query params (`sig`, `n`, `expire`).
- **`crates/core/src/innertube/client.rs`**:
  - Add `po_token_provider: RwLock<Option<Arc<dyn PoTokenProvider>>>` to `InnerTubeClient`.
  - Add builder `pub fn with_po_token_provider(self, provider: Arc<dyn PoTokenProvider>) -> Self` and setter `pub fn set_po_token_provider(&self, provider: Option<Arc<dyn PoTokenProvider>>)`.
  - In `resolve_stream`:
    - Determine `has_gvs_po = self.po_token_provider.read().unwrap().is_some() || self.gvs_po_token.read().unwrap().is_some()`.
    - Pass `has_gvs_po` into `client_matrix.playback_chain(tier, has_gvs_po, false, false)`.
  - In `resolve_with_client(client, video_id)`:
    - Look up `entry = self.client_matrix.read().unwrap().entry_for(client.kind).cloned()`.
    - Evaluate `needs_pot = client.allows_browser_cookies() && entry.is_some_and(|e| e.po_token_policy.requires_gvs_po_token(tier))`.
    - **Hard guard**: if `matches!(client.auth_mode, AuthMode::AnonymousOnly)` (`ANDROID_VR`, `IOS`), `needs_pot` is unconditionally `false`; never call `PoTokenProvider`, never attach `serviceIntegrityDimensions`, never append `&pot=`, never attach cookies/`SAPISIDHASH`.
    - When `needs_pot` is `true`:
      1. Obtain token from `po_token_provider.mint_token(&req).await` (or legacy `gvs_po_token`).
      2. Insert `"serviceIntegrityDimensions": { "poToken": token.value }` into `/player` request JSON.
      3. Append `&pot=<token.value>` to resolved `StreamInfo.url` via `attach_pot_param`.
    - When `needs_pot` is `false` (e.g. `PoTokenPolicy::NotRequired`, `Optional`, or Premium subscriber on `RequiredForWebFreeBypassedByPremium`): DO NOT call `PoTokenProvider` and DO NOT append `&pot=`.
- **`crates/core/src/innertube/auth.rs`**:
  - Extend `redact_secrets` to redact `pot=` query parameters and `poToken` JSON fields in diagnostic logs.
- **`src/live_engine.rs`**:
  - Wire optional `RESONANCE_POTOKEN_URL` (strictly `127.0.0.1` / `[::1]` only) into `CachingPoTokenProvider::new(SidecarPoTokenProvider::new(...))` on the background Tokio runtime (never on UI thread).

## 3. Security & Concurrency Edge Cases

- **`AuthMode::AnonymousOnly` isolation**: `ANDROID_VR` and `IOS` MUST NEVER receive browser cookies, `SAPISIDHASH`, or web GVS `&pot=` tokens — enforced both in `ClientMatrixEntry::is_eligible` and `resolve_with_client`.
- **SSRF prevention in `SidecarPoTokenProvider`**: Parse endpoint URL and strictly reject any host other than `127.0.0.1`, `::1`, or `localhost` so a poisoned env/config value cannot exfiltrate `visitorData` to remote servers.
- **Provider timeout / failure fallback**: Mint timeout (4s) or `Unavailable` error causes `resolve_with_client` on `WEB_REMIX` to fail cleanly so `resolve_stream` falls through to next matrix client (`VISIONOS` / `ANDROID_VR`) without stalling playback.
- **Stale PO token on HTTP 403**: Expose `InnerTubeClient::invalidate_po_token()` so `LoopbackProxy` 403 recovery (or retry path) evicts expired cached GVS tokens before re-resolving.
- **Single-flight concurrency**: `CachingPoTokenProvider` serializes concurrent cold mints for the same `(client_name, visitor_data, signed_in)` key via async lock so rapid queue pre-resolution + play never spawns duplicate sidecar/WebView mints.
- **Zero token leakage**: `PoToken.value` is never written to SQLite `metadata_cache` error strings or `log::*!` output (`Debug` impl masks `value` to `"[REDACTED_POT]"`).

## 4. Offline Verification Tests (`tests/potoken_policy_and_isolation.rs`)

Mocks: `MockPoTokenProvider` (tracks `mint_calls: AtomicUsize`, `last_request`, configurable delay/error) + local `127.0.0.1:0` `hyper` mock InnerTube server capturing incoming `/player` headers & JSON body.
1. `web_remix_required_policy_mints_and_attaches_pot_param_and_integrity_dim` — configures matrix with `WEB_REMIX` `PoTokenPolicy::Required`; asserts `mint_calls == 1`, `/player` JSON contains `serviceIntegrityDimensions.poToken`, and returned `StreamInfo.url` contains `&pot=MOCK_POT`.
2. `web_remix_free_bypassed_by_premium_skips_pot_for_premium_and_requires_for_free` — free/anonymous calls `mint_token` and attaches `&pot=`; flipping `set_premium_account(true)` makes `mint_calls == 0` and omits `&pot=`.
3. `android_vr_and_ios_never_receive_cookies_sapisidhash_or_pot` — enables signed-in session + `MockPoTokenProvider`, forces resolution through `ANDROID_VR` (and matrix-tested `IOS` guard); asserts zero `Cookie`, zero `Authorization`, `mint_calls == 0`, and no `pot=` on URL.
4. `not_required_and_optional_policies_never_invoke_provider_or_attach_pot` — resolves via `VISIONOS` (`Optional`) and `ANDROID_VR` (`NotRequired`); asserts `mint_calls == 0` and URL untouched.
5. `caching_potoken_provider_deduplicates_concurrent_mints_and_invalidates_on_403` — fires 8 concurrent `mint_token` calls -> asserts underlying provider called once (`mint_calls == 1`); calls `invalidate(PoTokenScope::Gvs)` -> next call remints (`mint_calls == 2`).
6. `sidecar_provider_rejects_non_loopback_urls_and_handles_timeout` — asserts `https://evil.example/get_pot` returns `Err(PoTokenError::Unavailable(_))`; stalled local server times out cleanly and falls back to `ANDROID_VR`.
7. `potoken_debug_and_redact_secrets_never_leak_token_value` — asserts `format!("{:?}", token)` and `redact_secrets(...)` never contain raw token plaintext.

```bash
cargo test --test potoken_policy_and_isolation -- --nocapture
cargo test --test architecture_gates_v2 --test core_unit_and_snapshots
```
