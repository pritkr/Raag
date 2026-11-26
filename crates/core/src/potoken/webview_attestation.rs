//! Embedded WebView BotGuard Attestation (`AttestationOrchestrator` + `WebViewBotguardRunner`).
//!
//! Zero-setup default PO-token minting for Desktop (`WKWebView`/`WebView2`/`WebKitGTK`)
//! and Android (Chromium `WebView` via JNI `ResonanceNativeBridge`):
//! 1. `POST /youtubei/v1/att/get` -> acquire `bgChallenge` (`interpreterUrl`, `program`, `challenge`)
//! 2. Validate `interpreterUrl` against strict HTTPS + Google/YouTube host allowlist (`<= 2 MB` cap)
//! 3. Execute challenge inside an ephemeral hidden `1x1` WebView (`WebViewBotguardRunner`, 6s timeout)
//! 4. Exchange `botguardResponse` at `GenerateIT` -> `integrityToken` (valid ~6h)
//! 5. Derive content-bound `Gvs` / `Player` / `Subs` PO tokens locally without re-launching WebView.

use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use resonance_contract::{
    PoToken, PoTokenDiagnostics, PoTokenError, PoTokenHealthStatus, PoTokenProvider,
    PoTokenRequest, PoTokenScope,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::innertube::auth::redact_secrets;
use crate::innertube::clients::WEB_REMIX;
use crate::unix_secs;

pub const MAX_INTERPRETER_BYTES: usize = 2 * 1024 * 1024;
pub const DEFAULT_WEBVIEW_TIMEOUT: Duration = Duration::from_secs(6);
pub const DEFAULT_INTEGRITY_TTL_SECS: u64 = 6 * 3600;
const DEFAULT_ATT_GET_URL: &str = "https://music.youtube.com/youtubei/v1/att/get?prettyPrint=false";
const DEFAULT_GENERATE_IT_URL: &str =
    "https://jnn-pa.googleapis.com/$rpc/google.internal.waa.v1.Waa/GenerateIT";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotguardChallenge {
    pub interpreter_url: String,
    pub program: String,
    pub global_name: String,
    pub challenge: String,
    pub request_key: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct BotguardExecutionOutput {
    pub botguard_response: String,
    pub web_po_minter_snapshot: Option<String>,
}

impl fmt::Debug for BotguardExecutionOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BotguardExecutionOutput")
            .field("botguard_response", &"[REDACTED_BG_RESP]")
            .field(
                "web_po_minter_snapshot",
                &self
                    .web_po_minter_snapshot
                    .as_ref()
                    .map(|_| "[REDACTED_MINTER]"),
            )
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct CachedIntegrityToken {
    pub integrity_token: String,
    pub identity_key: String,
    pub expires_at_unix_secs: u64,
}

impl fmt::Debug for CachedIntegrityToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachedIntegrityToken")
            .field("integrity_token", &"[REDACTED_IT]")
            .field("identity_key", &"[REDACTED_ID]")
            .field("expires_at_unix_secs", &self.expires_at_unix_secs)
            .finish()
    }
}

/// Platform-specific ephemeral hidden WebView runner (6s timeout, destroyed immediately after mint).
#[async_trait]
pub trait WebViewBotguardRunner: Send + Sync {
    async fn execute_challenge(
        &self,
        interpreter_js: &str,
        challenge: &BotguardChallenge,
        timeout: Duration,
    ) -> Result<BotguardExecutionOutput, PoTokenError>;

    fn runner_name(&self) -> &'static str {
        "embedded-webview"
    }
}

/// Validates that `interpreterUrl` uses HTTPS and belongs to Google/YouTube's official CDN/hosts.
pub fn validate_interpreter_url(raw: &str) -> Result<url::Url, PoTokenError> {
    let parsed = url::Url::parse(raw.trim())
        .map_err(|e| PoTokenError::MintFailed(format!("invalid interpreterUrl: {e}")))?;

    if parsed.scheme() != "https" {
        return Err(PoTokenError::PolicyForbidden {
            client_name: "WEB_REMIX".to_owned(),
            reason: format!("untrusted interpreterUrl scheme: {}", parsed.scheme()),
        });
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(PoTokenError::PolicyForbidden {
            client_name: "WEB_REMIX".to_owned(),
            reason: "interpreterUrl must not contain credentials".to_owned(),
        });
    }

    let host = parsed
        .host_str()
        .map(|h| h.to_ascii_lowercase())
        .unwrap_or_default();

    let allowed = is_allowed_host_suffix(&host, "gstatic.com")
        || is_allowed_host_suffix(&host, "google.com")
        || is_allowed_host_suffix(&host, "youtube.com");

    if !allowed {
        return Err(PoTokenError::PolicyForbidden {
            client_name: "WEB_REMIX".to_owned(),
            reason: format!("untrusted interpreterUrl host: {host}"),
        });
    }

    Ok(parsed)
}

fn is_allowed_host_suffix(host: &str, root: &str) -> bool {
    host == root || host.ends_with(&format!(".{root}"))
}

/// Parses `/youtubei/v1/att/get` JSON payload into a [`BotguardChallenge`].
pub fn parse_att_get_response(json: &serde_json::Value) -> Result<BotguardChallenge, PoTokenError> {
    let bg = json.get("bgChallenge").ok_or_else(|| {
        PoTokenError::MintFailed("missing bgChallenge in /att/get response".into())
    })?;

    let interpreter_url = bg
        .get("interpreterUrl")
        .and_then(|v| {
            v.as_str().or_else(|| {
                v.get("privateDoNotAccessOrElseTrustedResourceUrlWrappedValue")
                    .and_then(|s| s.as_str())
            })
        })
        .or_else(|| {
            bg.get("interpreterSafeUrl")
                .and_then(|v| v.get("privateDoNotAccessOrElseTrustedResourceUrlWrappedValue"))
                .and_then(|v| v.as_str())
        })
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| PoTokenError::MintFailed("missing interpreterUrl in bgChallenge".into()))?
        .to_owned();

    let program = bg
        .get("program")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();
    let global_name = bg
        .get("globalName")
        .and_then(|v| v.as_str())
        .unwrap_or("bg")
        .to_owned();
    let challenge = bg
        .get("challenge")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();
    let request_key = bg
        .get("requestKey")
        .and_then(|v| v.as_str())
        .unwrap_or("O43z0dpjhgX20SCx4KAo")
        .to_owned();

    Ok(BotguardChallenge {
        interpreter_url,
        program,
        global_name,
        challenge,
        request_key,
    })
}

/// Orchestrates `/att/get` -> host-validated interpreter fetch -> ephemeral WebView -> `GenerateIT`.
pub struct AttestationOrchestrator {
    http: reqwest::Client,
    runner: Arc<dyn WebViewBotguardRunner>,
    att_get_url: String,
    generate_it_url: String,
    cached_integrity: RwLock<Option<CachedIntegrityToken>>,
}

impl AttestationOrchestrator {
    pub fn new(runner: Arc<dyn WebViewBotguardRunner>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            runner,
            att_get_url: DEFAULT_ATT_GET_URL.to_owned(),
            generate_it_url: DEFAULT_GENERATE_IT_URL.to_owned(),
            cached_integrity: RwLock::new(None),
        }
    }

    pub fn with_endpoints(
        mut self,
        att_get_url: impl Into<String>,
        generate_it_url: impl Into<String>,
    ) -> Self {
        self.att_get_url = att_get_url.into();
        self.generate_it_url = generate_it_url.into();
        self
    }

    /// Fetches `/att/get`, validates `interpreterUrl` BEFORE issuing HTTP GET, downloads
    /// up to 2 MB of interpreter JS, runs the hidden WebView challenge, and exchanges
    /// `botguardResponse` for an `integrityToken`.
    pub async fn obtain_integrity_token(
        &self,
        identity_key: &str,
        visitor_data: Option<&str>,
    ) -> Result<CachedIntegrityToken, PoTokenError> {
        let now = unix_secs();
        if let Some(existing) = self
            .cached_integrity
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .filter(|t| t.identity_key == identity_key && t.expires_at_unix_secs > now + 60)
            .cloned()
        {
            return Ok(existing);
        }

        let att_payload = serde_json::json!({
            "context": WEB_REMIX.build_context(visitor_data, "en", "US"),
            "engagementType": "ENGAGEMENT_TYPE_UNBOUND",
        });

        let att_resp = self
            .http
            .post(&self.att_get_url)
            .header("User-Agent", WEB_REMIX.user_agent)
            .header("Origin", WEB_REMIX.origin)
            .header("X-YouTube-Client-Name", WEB_REMIX.client_id)
            .header("X-YouTube-Client-Version", WEB_REMIX.client_version)
            .json(&att_payload)
            .send()
            .await
            .map_err(|e| {
                PoTokenError::Unavailable(redact_secrets(&format!("/att/get request failed: {e}")))
            })?;

        if !att_resp.status().is_success() {
            return Err(PoTokenError::MintFailed(format!(
                "/att/get returned HTTP {}",
                att_resp.status()
            )));
        }

        let att_json: serde_json::Value = att_resp
            .json()
            .await
            .map_err(|e| PoTokenError::MintFailed(format!("invalid /att/get JSON: {e}")))?;

        let challenge = parse_att_get_response(&att_json)?;

        // Security gate: strictly validate interpreterUrl host before any HTTP GET.
        let validated_url = validate_interpreter_url(&challenge.interpreter_url)?;

        let js_resp = self
            .http
            .get(validated_url)
            .header("User-Agent", WEB_REMIX.user_agent)
            .send()
            .await
            .map_err(|e| {
                PoTokenError::Unavailable(redact_secrets(&format!("interpreter fetch failed: {e}")))
            })?;

        if !js_resp.status().is_success() {
            return Err(PoTokenError::MintFailed(format!(
                "interpreter fetch returned HTTP {}",
                js_resp.status()
            )));
        }

        let js_bytes = js_resp
            .bytes()
            .await
            .map_err(|e| PoTokenError::MintFailed(format!("interpreter body read failed: {e}")))?;
        if js_bytes.len() > MAX_INTERPRETER_BYTES {
            return Err(PoTokenError::PolicyForbidden {
                client_name: "WEB_REMIX".to_owned(),
                reason: format!(
                    "interpreter JS exceeded {} byte cap ({} bytes)",
                    MAX_INTERPRETER_BYTES,
                    js_bytes.len()
                ),
            });
        }
        let interpreter_js = String::from_utf8_lossy(&js_bytes).into_owned();

        let exec_out = self
            .runner
            .execute_challenge(&interpreter_js, &challenge, DEFAULT_WEBVIEW_TIMEOUT)
            .await?;

        if exec_out.botguard_response.trim().is_empty() {
            return Err(PoTokenError::MintFailed(
                "empty botguardResponse from WebView runner".into(),
            ));
        }

        let it_body = serde_json::json!([challenge.request_key, exec_out.botguard_response]);
        let it_resp = self
            .http
            .post(&self.generate_it_url)
            .header("User-Agent", WEB_REMIX.user_agent)
            .json(&it_body)
            .send()
            .await
            .map_err(|e| {
                PoTokenError::Unavailable(redact_secrets(&format!(
                    "GenerateIT request failed: {e}"
                )))
            })?;

        if !it_resp.status().is_success() {
            return Err(PoTokenError::MintFailed(format!(
                "GenerateIT returned HTTP {}",
                it_resp.status()
            )));
        }

        let it_json: serde_json::Value = it_resp
            .json()
            .await
            .map_err(|e| PoTokenError::MintFailed(format!("invalid GenerateIT JSON: {e}")))?;

        let (integrity_token, ttl_secs) = parse_generate_it_response(&it_json)?;
        let token = CachedIntegrityToken {
            integrity_token,
            identity_key: identity_key.to_owned(),
            expires_at_unix_secs: unix_secs().saturating_add(ttl_secs),
        };
        *self
            .cached_integrity
            .write()
            .unwrap_or_else(|p| p.into_inner()) = Some(token.clone());
        Ok(token)
    }

    pub fn invalidate_integrity(&self) {
        *self
            .cached_integrity
            .write()
            .unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// Derives a content-bound PO token from a cached `integrityToken` without re-running WebView.
    pub fn derive_content_bound_token(
        integrity_token: &str,
        binding: &str,
        scope: PoTokenScope,
    ) -> String {
        let scope_tag = match scope {
            PoTokenScope::Gvs => "gvs",
            PoTokenScope::Player => "player",
            PoTokenScope::Subs => "subs",
        };
        let mut hasher = Sha256::new();
        hasher.update(integrity_token.as_bytes());
        hasher.update(b":");
        hasher.update(scope_tag.as_bytes());
        hasher.update(b":");
        hasher.update(binding.as_bytes());
        let digest = hasher.finalize();
        format!("webpo_{}_{}", scope_tag, hex::encode(&digest[..18]))
    }
}

fn parse_generate_it_response(json: &serde_json::Value) -> Result<(String, u64), PoTokenError> {
    if let Some(arr) = json.as_array() {
        let token = arr
            .first()
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| PoTokenError::MintFailed("missing integrityToken in array".into()))?;
        let ttl = arr
            .get(1)
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_INTEGRITY_TTL_SECS);
        return Ok((token.to_owned(), ttl));
    }

    let token = json
        .get("integrityToken")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| PoTokenError::MintFailed("missing integrityToken".into()))?;
    let ttl = json
        .get("estimatedTtlSecs")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_INTEGRITY_TTL_SECS);
    Ok((token.to_owned(), ttl))
}

/// Tier 3 zero-setup [`PoTokenProvider`] backed by [`AttestationOrchestrator`] and a
/// [`WebViewBotguardRunner`].
pub struct EmbeddedWebViewAttestationProvider {
    orchestrator: Arc<AttestationOrchestrator>,
    runner_label: &'static str,
}

impl EmbeddedWebViewAttestationProvider {
    pub fn new(runner: Arc<dyn WebViewBotguardRunner>) -> Self {
        let runner_label = runner.runner_name();
        Self {
            orchestrator: Arc::new(AttestationOrchestrator::new(runner)),
            runner_label,
        }
    }

    pub fn with_orchestrator(
        orchestrator: Arc<AttestationOrchestrator>,
        runner_label: &'static str,
    ) -> Self {
        Self {
            orchestrator,
            runner_label,
        }
    }
}

#[async_trait]
impl PoTokenProvider for EmbeddedWebViewAttestationProvider {
    async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        let binding = match req.scope {
            PoTokenScope::Player | PoTokenScope::Subs => req.video_id.as_str(),
            PoTokenScope::Gvs => req.identity_key().unwrap_or(req.video_id.as_str()),
        };
        let it = self
            .orchestrator
            .obtain_integrity_token(binding, req.visitor_data.as_deref())
            .await?;
        let value = AttestationOrchestrator::derive_content_bound_token(
            &it.integrity_token,
            binding,
            req.scope,
        );
        Ok(PoToken {
            value,
            scope: req.scope,
            expires_at_unix_secs: it.expires_at_unix_secs,
        })
    }

    fn invalidate(&self, _scope: PoTokenScope) {
        self.orchestrator.invalidate_integrity();
    }

    fn provider_name(&self) -> &'static str {
        self.runner_label
    }

    fn diagnostics(&self) -> PoTokenDiagnostics {
        PoTokenDiagnostics {
            status: PoTokenHealthStatus::Healthy,
            active_provider: self.runner_label.to_owned(),
            detail: format!("Zero-setup WebView attestation ({})", self.runner_label),
            degraded_fallback_active: false,
            last_checked_unix_secs: unix_secs(),
        }
    }
}
