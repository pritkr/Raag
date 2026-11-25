//! Embedded WebView / Platform Attestation Bridge for BotGuard PO Token minting.
//!
//! Supports:
//! - Android WebView / Play Integrity BotGuard attestation bridge (`platform/android`)
//! - Desktop headless WebView or bundled QuickJS-NG VM fallback (`rquickjs`)

use std::sync::Arc;

use async_trait::async_trait;
use resonance_contract::{
    PoToken, PoTokenDiagnostics, PoTokenError, PoTokenHealthStatus, PoTokenProvider,
    PoTokenRequest, PoTokenScope,
};
use rquickjs::{Context, Function, Runtime};

use crate::unix_secs;

const DEFAULT_ATTESTATION_TTL_SECS: u64 = 4 * 3600;

/// Platform-specific attestation hook implemented by Android (WebView/Play Integrity JNI)
/// or Desktop (headless WebView / native bridge).
#[async_trait]
pub trait PlatformAttestationBridge: Send + Sync {
    async fn mint_attested_token(
        &self,
        binding: &str,
        scope: PoTokenScope,
    ) -> Result<(String, u64), PoTokenError>;

    fn bridge_kind(&self) -> &'static str;
}

/// Optional bundled QuickJS-NG JS VM fallback bridge for desktop/offline attestation scripts.
///
/// Executes an operator-supplied BotGuard/attestation JS module defining
/// `function mintPoToken(binding, scope) -> string` inside an isolated 32 MB QuickJS-NG VM.
pub struct QuickJsVmAttestationBridge {
    script_source: String,
}

impl QuickJsVmAttestationBridge {
    pub fn new(script_source: impl Into<String>) -> Self {
        Self {
            script_source: script_source.into(),
        }
    }
}

#[async_trait]
impl PlatformAttestationBridge for QuickJsVmAttestationBridge {
    async fn mint_attested_token(
        &self,
        binding: &str,
        scope: PoTokenScope,
    ) -> Result<(String, u64), PoTokenError> {
        let source = self.script_source.clone();
        let binding_owned = binding.to_owned();
        let scope_label = match scope {
            PoTokenScope::Gvs => "gvs",
            PoTokenScope::Player => "player",
            PoTokenScope::Subs => "subs",
        }
        .to_owned();

        let token = tokio::task::spawn_blocking(move || -> Result<String, PoTokenError> {
            let rt = Runtime::new()
                .map_err(|e| PoTokenError::MintFailed(format!("QuickJS VM init failed: {e}")))?;
            rt.set_memory_limit(32 * 1024 * 1024);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            rt.set_interrupt_handler(Some(Box::new(move || {
                std::time::Instant::now() >= deadline
            })));
            let ctx = Context::full(&rt)
                .map_err(|e| PoTokenError::MintFailed(format!("QuickJS context failed: {e}")))?;
            ctx.with(|ctx| {
                ctx.eval::<(), _>(source.as_bytes())
                    .map_err(|e| PoTokenError::MintFailed(format!("VM eval failed: {e}")))?;
                let func: Function = ctx
                    .globals()
                    .get("mintPoToken")
                    .map_err(|e| PoTokenError::MintFailed(format!("missing mintPoToken: {e}")))?;
                let out: String = func
                    .call((binding_owned, scope_label))
                    .map_err(|e| PoTokenError::MintFailed(format!("mintPoToken failed: {e}")))?;
                if out.trim().is_empty() {
                    return Err(PoTokenError::MintFailed("empty token from JS VM".into()));
                }
                Ok(out)
            })
        })
        .await
        .map_err(|e| PoTokenError::MintFailed(format!("attestation worker panicked: {e}")))??;

        Ok((token, unix_secs() + DEFAULT_ATTESTATION_TTL_SECS))
    }

    fn bridge_kind(&self) -> &'static str {
        "embedded-quickjs-vm"
    }
}

/// [`PoTokenProvider`] backed by a [`PlatformAttestationBridge`] (Android WebView/Play Integrity
/// or Desktop headless WebView / QuickJS-NG VM).
pub struct PlatformAttestationPoTokenProvider {
    bridge: Arc<dyn PlatformAttestationBridge>,
}

impl PlatformAttestationPoTokenProvider {
    pub fn new(bridge: Arc<dyn PlatformAttestationBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl PoTokenProvider for PlatformAttestationPoTokenProvider {
    async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        let binding = match req.scope {
            PoTokenScope::Player | PoTokenScope::Subs => req.video_id.as_str(),
            PoTokenScope::Gvs => req.identity_key().unwrap_or(req.video_id.as_str()),
        };
        let (value, expires_at_unix_secs) =
            self.bridge.mint_attested_token(binding, req.scope).await?;
        Ok(PoToken {
            value,
            scope: req.scope,
            expires_at_unix_secs,
        })
    }

    fn invalidate(&self, _scope: PoTokenScope) {}

    fn provider_name(&self) -> &'static str {
        self.bridge.bridge_kind()
    }

    fn diagnostics(&self) -> PoTokenDiagnostics {
        PoTokenDiagnostics {
            status: PoTokenHealthStatus::Healthy,
            active_provider: self.bridge.bridge_kind().to_owned(),
            detail: format!(
                "Platform attestation bridge ({})",
                self.bridge.bridge_kind()
            ),
            degraded_fallback_active: false,
            last_checked_unix_secs: unix_secs(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn quickjs_vm_attestation_bridge_mints_scoped_tokens() {
        let bridge = Arc::new(QuickJsVmAttestationBridge::new(
            "function mintPoToken(binding, scope) { return 'vm_' + scope + '_' + binding; }",
        ));
        let provider = PlatformAttestationPoTokenProvider::new(bridge);
        let req = PoTokenRequest::new(
            "WEB_REMIX",
            PoTokenScope::Subs,
            "vid_42",
            Some("vd_99".into()),
            false,
        );
        let tok = provider.mint_token(&req).await.unwrap();
        assert_eq!(tok.value, "vm_subs_vid_42");
        assert_eq!(tok.scope, PoTokenScope::Subs);
    }
}
