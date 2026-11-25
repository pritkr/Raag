//! Layered PO Token Provider Chain with automatic fallback & cold-start degraded diagnostics.
//!
//! Resolution order:
//! 1. External/Local HTTP PO Provider (`bgutil-ytdlp-pot-provider`)
//! 2. Embedded WebView / Platform Attestation Bridge (Android Play Integrity / Desktop JS VM)
//! 3. Cold-Start Degraded Fallback (signals `PoTokenHealthStatus::DegradedColdStart` so
//!    `InnerTubeClient` shifts cleanly to lower PO-enforcement clients in `client_matrix.json`
//!    and Settings displays a non-blocking diagnostics badge).

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use resonance_contract::{
    PoToken, PoTokenDiagnostics, PoTokenError, PoTokenHealthStatus, PoTokenProvider,
    PoTokenRequest, PoTokenScope,
};

use crate::unix_secs;

/// Layered chain of [`PoTokenProvider`]s with automatic failover and health tracking.
pub struct ChainedPoTokenProvider {
    providers: Vec<Arc<dyn PoTokenProvider>>,
    diagnostics: RwLock<PoTokenDiagnostics>,
}

impl ChainedPoTokenProvider {
    pub fn new(providers: Vec<Arc<dyn PoTokenProvider>>) -> Self {
        let initial = if providers.is_empty() {
            PoTokenDiagnostics {
                status: PoTokenHealthStatus::Unconfigured,
                active_provider: "none".to_owned(),
                detail: "No PO token provider configured — using fallback matrix clients".to_owned(),
                degraded_fallback_active: true,
                last_checked_unix_secs: unix_secs(),
            }
        } else {
            let names: Vec<&str> = providers.iter().map(|p| p.provider_name()).collect();
            PoTokenDiagnostics {
                status: PoTokenHealthStatus::Healthy,
                active_provider: names[0].to_owned(),
                detail: format!("Chain ready: {}", names.join(" -> ")),
                degraded_fallback_active: false,
                last_checked_unix_secs: unix_secs(),
            }
        };
        Self {
            providers,
            diagnostics: RwLock::new(initial),
        }
    }

    pub fn is_degraded(&self) -> bool {
        self.diagnostics.read().unwrap().degraded_fallback_active
    }
}

#[async_trait]
impl PoTokenProvider for ChainedPoTokenProvider {
    async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        let mut last_err = None;
        for provider in &self.providers {
            match provider.mint_token(req).await {
                Ok(token) => {
                    *self.diagnostics.write().unwrap() = PoTokenDiagnostics {
                        status: PoTokenHealthStatus::Healthy,
                        active_provider: provider.provider_name().to_owned(),
                        detail: format!("Minting via {}", provider.provider_name()),
                        degraded_fallback_active: false,
                        last_checked_unix_secs: unix_secs(),
                    };
                    return Ok(token);
                }
                Err(e) => {
                    let safe_err = crate::innertube::auth::redact_secrets(&e.to_string());
                    log::warn!(
                        "PO token provider '{}' failed ({safe_err}); trying next provider in chain",
                        provider.provider_name()
                    );
                    last_err = Some(e);
                }
            }
        }

        let err = last_err.unwrap_or_else(|| {
            PoTokenError::Unavailable("all po token providers unavailable".to_owned())
        });
        let safe_err = crate::innertube::auth::redact_secrets(&err.to_string());
        *self.diagnostics.write().unwrap() = PoTokenDiagnostics {
            status: PoTokenHealthStatus::DegradedColdStart,
            active_provider: "degraded-fallback".to_owned(),
            detail: format!("PO mint degraded ({safe_err}); using non-PO fallback clients"),
            degraded_fallback_active: true,
            last_checked_unix_secs: unix_secs(),
        };
        Err(err)
    }

    fn invalidate(&self, scope: PoTokenScope) {
        for p in &self.providers {
            p.invalidate(scope);
        }
    }

    fn provider_name(&self) -> &'static str {
        "layered-chain"
    }

    fn diagnostics(&self) -> PoTokenDiagnostics {
        self.diagnostics.read().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::potoken::attestation::{
        PlatformAttestationPoTokenProvider, QuickJsVmAttestationBridge,
    };
    use crate::potoken::NoopPoTokenProvider;

    #[tokio::test]
    async fn falls_back_from_failing_primary_to_attestation_and_tracks_degraded_state() {
        let vm = Arc::new(QuickJsVmAttestationBridge::new(
            "function mintPoToken(b, s) { return 'ok_' + s + '_' + b; }",
        ));
        let chain = ChainedPoTokenProvider::new(vec![
            Arc::new(NoopPoTokenProvider),
            Arc::new(PlatformAttestationPoTokenProvider::new(vm)),
        ]);

        let req = PoTokenRequest::new("WEB_REMIX", PoTokenScope::Gvs, "v1", Some("vd_1".into()), false);
        let tok = chain.mint_token(&req).await.unwrap();
        assert_eq!(tok.value, "ok_gvs_vd_1");
        assert!(!chain.is_degraded());
        assert_eq!(chain.diagnostics().status, PoTokenHealthStatus::Healthy);
        assert_eq!(chain.diagnostics().active_provider, "embedded-quickjs-vm");

        // All failing -> cold-start degraded mode
        let degraded_chain = ChainedPoTokenProvider::new(vec![Arc::new(NoopPoTokenProvider)]);
        assert!(degraded_chain.mint_token(&req).await.is_err());
        assert!(degraded_chain.is_degraded());
        assert_eq!(
            degraded_chain.diagnostics().status,
            PoTokenHealthStatus::DegradedColdStart
        );
    }
}
