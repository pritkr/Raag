//! Offline verification tests for W3A deltas:
//! - Hard AuthMode::AnonymousOnly isolation (`ANDROID_VR` and `IOS` never receive Web PO tokens)
//! - Per-tier 10-minute cooldown skipping failing Tier 1 on subsequent calls
//! - `redact_secrets` scrubbing `pot=` and `poToken` material

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use resonance_contract::{PoToken, PoTokenError, PoTokenProvider, PoTokenRequest, PoTokenScope};
use resonance_core::innertube::auth::redact_secrets;
use resonance_core::innertube::clients::AuthMode;
use resonance_core::potoken::ChainedPoTokenProvider;

struct CountingFailProvider {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl PoTokenProvider for CountingFailProvider {
    async fn mint_token(&self, _req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(PoTokenError::Unavailable("tier1 timed out".to_owned()))
    }
    fn invalidate(&self, _scope: PoTokenScope) {}
    fn provider_name(&self) -> &'static str {
        "tier1-failing"
    }
}

struct HealthyMockProvider {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl PoTokenProvider for HealthyMockProvider {
    async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(PoToken {
            value: "mock_pot_token_ok".to_owned(),
            scope: req.scope,
            expires_at_unix_secs: u64::MAX,
        })
    }
    fn invalidate(&self, _scope: PoTokenScope) {}
    fn provider_name(&self) -> &'static str {
        "tier2-healthy"
    }
}

#[tokio::test]
async fn anonymous_only_clients_never_receive_web_potokens() {
    let healthy = Arc::new(HealthyMockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
    });
    let chain = ChainedPoTokenProvider::new(vec![healthy.clone()]);
    let req = PoTokenRequest::new(
        "WEB_REMIX",
        PoTokenScope::Gvs,
        "vid_1",
        Some("vd_1".to_owned()),
        false,
    );

    // ANDROID_VR and IOS MUST return Ok(None) with 0 mint calls.
    assert_eq!(
        chain
            .resolve_for_client("ANDROID_VR", AuthMode::AnonymousOnly, &req)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        chain
            .resolve_for_client("IOS", AuthMode::AnonymousOnly, &req)
            .await
            .unwrap(),
        None
    );
    assert_eq!(healthy.calls.load(Ordering::SeqCst), 0);

    // WEB_REMIX receives the token normally.
    let web_tok = chain
        .resolve_for_client("WEB_REMIX", AuthMode::CookiesOrAnonymous, &req)
        .await
        .unwrap();
    assert!(web_tok.is_some());
    assert_eq!(healthy.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn chained_potoken_trips_tier_cooldown_on_failure_and_skips_to_next_tier() {
    let tier1_calls = Arc::new(AtomicUsize::new(0));
    let tier2_calls = Arc::new(AtomicUsize::new(0));
    let chain = ChainedPoTokenProvider::new(vec![
        Arc::new(CountingFailProvider {
            calls: Arc::clone(&tier1_calls),
        }),
        Arc::new(HealthyMockProvider {
            calls: Arc::clone(&tier2_calls),
        }),
    ]);

    let req = PoTokenRequest::new(
        "WEB_REMIX",
        PoTokenScope::Gvs,
        "vid_1",
        Some("vd_1".to_owned()),
        false,
    );

    // 1st call: Tier 1 fails -> falls back to Tier 2 -> Tier 1 enters 10m cooldown.
    let tok1 = chain.mint_token(&req).await.unwrap();
    assert_eq!(tok1.value, "mock_pot_token_ok");
    assert_eq!(tier1_calls.load(Ordering::SeqCst), 1);
    assert_eq!(tier2_calls.load(Ordering::SeqCst), 1);

    // 2nd call: Tier 1 is skipped (still 1 call), Tier 2 serves immediately.
    let tok2 = chain.mint_token(&req).await.unwrap();
    assert_eq!(tok2.value, "mock_pot_token_ok");
    assert_eq!(tier1_calls.load(Ordering::SeqCst), 1);
    assert_eq!(tier2_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn potoken_redaction_strips_sensitive_token_material() {
    let raw = "https://rr1---sn.googlevideo.com/videoplayback?itag=141&pot=SUPER_SECRET_POT_12345";
    let redacted = redact_secrets(raw);
    assert!(!redacted.contains("SUPER_SECRET_POT_12345"));
}
