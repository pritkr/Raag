//! Embedded QuickJS-NG VM attestation fallback for BotGuard PO Token minting.
//!
//! Executes an operator-supplied BotGuard/attestation JS module defining
//! `function mintPoToken(binding, scope) -> string` inside an isolated 32 MB QuickJS-NG VM.

use async_trait::async_trait;
use resonance_contract::{
    PoToken, PoTokenDiagnostics, PoTokenError, PoTokenHealthStatus, PoTokenProvider,
    PoTokenRequest, PoTokenScope,
};
use rquickjs::{Context, Function, Runtime};

use crate::unix_secs;

const DEFAULT_ATTESTATION_TTL_SECS: u64 = 4 * 3600;

/// Bundled QuickJS-NG JS VM [`PoTokenProvider`] for desktop/offline attestation scripts.
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
impl PoTokenProvider for QuickJsVmAttestationBridge {
    async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        let binding = match req.scope {
            PoTokenScope::Player | PoTokenScope::Subs => req.video_id.as_str(),
            PoTokenScope::Gvs => req.identity_key().unwrap_or(req.video_id.as_str()),
        };
        let source = self.script_source.clone();
        let binding_owned = binding.to_owned();
        let scope_label = match req.scope {
            PoTokenScope::Gvs => "gvs",
            PoTokenScope::Player => "player",
            PoTokenScope::Subs => "subs",
        }
        .to_owned();

        let value = tokio::task::spawn_blocking(move || -> Result<String, PoTokenError> {
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

        Ok(PoToken {
            value,
            scope: req.scope,
            expires_at_unix_secs: unix_secs() + DEFAULT_ATTESTATION_TTL_SECS,
        })
    }

    fn invalidate(&self, _scope: PoTokenScope) {}

    fn provider_name(&self) -> &'static str {
        "embedded-quickjs-vm"
    }

    fn diagnostics(&self) -> PoTokenDiagnostics {
        PoTokenDiagnostics {
            status: PoTokenHealthStatus::Healthy,
            active_provider: self.provider_name().to_owned(),
            detail: "Embedded QuickJS-NG attestation VM".to_owned(),
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
        let provider = QuickJsVmAttestationBridge::new(
            "function mintPoToken(binding, scope) { return 'vm_' + scope + '_' + binding; }",
        );
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
