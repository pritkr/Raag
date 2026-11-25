//! TTL-aware PO token cache with single-flight deduplication keyed by
//! `(visitor_data | session_hash, video_id, scope)` + `client_name`.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use resonance_contract::{
    PoToken, PoTokenDiagnostics, PoTokenError, PoTokenProvider, PoTokenRequest, PoTokenScope,
};
use tokio::sync::Mutex;

use crate::unix_secs;

pub const CACHE_EXPIRY_SKEW_SECS: u64 = 60;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CacheKey {
    pub scope: PoTokenScope,
    pub client_name: String,
    pub identity: Option<String>,
    pub signed_in: bool,
    pub video_id: Option<String>,
}

impl CacheKey {
    pub(crate) fn from_request(req: &PoTokenRequest) -> Self {
        Self {
            scope: req.scope,
            client_name: req.client_name.clone(),
            identity: req.identity_key().map(str::to_owned),
            signed_in: req.signed_in,
            video_id: match req.scope {
                PoTokenScope::Player | PoTokenScope::Subs => Some(req.video_id.clone()),
                PoTokenScope::Gvs => None,
            },
        }
    }
}

/// Caching + single-flight deduplicating wrapper around any [`PoTokenProvider`].
///
/// Keyed by `(visitor_data | session_hash, video_id, scope)` with a 60s expiry
/// skew margin and async single-flight serialization for concurrent cold mints.
pub struct CachingPoTokenProvider<P: PoTokenProvider> {
    inner: P,
    cache: RwLock<HashMap<CacheKey, PoToken>>,
    mint_lock: Mutex<()>,
}

impl<P: PoTokenProvider> CachingPoTokenProvider<P> {
    pub fn new(inner: P) -> Self {
        Self {
            inner,
            cache: RwLock::new(HashMap::new()),
            mint_lock: Mutex::new(()),
        }
    }

    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }

    pub fn inner(&self) -> &P {
        &self.inner
    }

    fn lookup_fresh(&self, key: &CacheKey) -> Option<PoToken> {
        let now = unix_secs();
        let guard = self.cache.read().unwrap_or_else(|p| p.into_inner());
        guard
            .get(key)
            .filter(|tok| tok.expires_at_unix_secs > now.saturating_add(CACHE_EXPIRY_SKEW_SECS))
            .cloned()
    }
}

#[async_trait]
impl<P: PoTokenProvider> PoTokenProvider for CachingPoTokenProvider<P> {
    async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        let key = CacheKey::from_request(req);
        if let Some(cached) = self.lookup_fresh(&key) {
            return Ok(cached);
        }

        let _guard = self.mint_lock.lock().await;
        if let Some(cached) = self.lookup_fresh(&key) {
            return Ok(cached);
        }

        let minted = self.inner.mint_token(req).await?;
        self.cache
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key, minted.clone());
        Ok(minted)
    }

    fn invalidate(&self, scope: PoTokenScope) {
        self.cache
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|k, _| k.scope != scope);
        self.inner.invalidate(scope);
    }

    fn provider_name(&self) -> &'static str {
        self.inner.provider_name()
    }

    fn diagnostics(&self) -> PoTokenDiagnostics {
        self.inner.diagnostics()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_separates_gvs_player_subs_and_session_hashes() {
        let gvs_a = PoTokenRequest::new(
            "WEB_REMIX",
            PoTokenScope::Gvs,
            "v1",
            Some("vd".into()),
            true,
        )
        .with_session_hash(Some("sess_a".into()));
        let gvs_b = PoTokenRequest::new(
            "WEB_REMIX",
            PoTokenScope::Gvs,
            "v2",
            Some("vd".into()),
            true,
        )
        .with_session_hash(Some("sess_a".into()));
        let gvs_other = PoTokenRequest::new(
            "WEB_REMIX",
            PoTokenScope::Gvs,
            "v1",
            Some("vd".into()),
            true,
        )
        .with_session_hash(Some("sess_b".into()));
        let subs_v1 = PoTokenRequest::new(
            "WEB_REMIX",
            PoTokenScope::Subs,
            "v1",
            Some("vd".into()),
            false,
        );
        let subs_v2 = PoTokenRequest::new(
            "WEB_REMIX",
            PoTokenScope::Subs,
            "v2",
            Some("vd".into()),
            false,
        );

        // Same session hash shares GVS token across videos
        assert_eq!(
            CacheKey::from_request(&gvs_a),
            CacheKey::from_request(&gvs_b)
        );
        // Different session hashes never collide
        assert_ne!(
            CacheKey::from_request(&gvs_a),
            CacheKey::from_request(&gvs_other)
        );
        // Subs scope is bound per video_id
        assert_ne!(
            CacheKey::from_request(&subs_v1),
            CacheKey::from_request(&subs_v2)
        );
    }
}
