//! TTL-aware PO token cache with:
//! - Per-key async single-flight deduplication (`tokio::sync::watch`)
//! - Proactive background refresh at 80% TTL (0 ms playback stall)
//! - Exponential negative-cache backoff (30s -> 5m) on total chain failure
//! - 10s debounced 403 invalidation

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use resonance_contract::{
    PoToken, PoTokenDiagnostics, PoTokenError, PoTokenProvider, PoTokenRequest, PoTokenScope,
};
use tokio::sync::{Mutex, watch};

use crate::unix_secs;

pub const CACHE_EXPIRY_SKEW_SECS: u64 = 60;
pub const INITIAL_NEGATIVE_BACKOFF_SECS: u64 = 30;
pub const MAX_NEGATIVE_BACKOFF_SECS: u64 = 300;
pub const INVALIDATE_DEBOUNCE_SECS: u64 = 10;

pub type ClockFn = Arc<dyn Fn() -> u64 + Send + Sync>;

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

#[derive(Debug, Clone)]
struct CacheEntry {
    token: PoToken,
    minted_at_unix_secs: u64,
    refreshing: bool,
}

#[derive(Debug, Clone, Copy)]
struct NegativeEntry {
    failed_at_unix_secs: u64,
    backoff_secs: u64,
}

#[derive(Default)]
struct CacheState {
    entries: HashMap<CacheKey, CacheEntry>,
    negative: HashMap<CacheKey, NegativeEntry>,
    last_invalidated: HashMap<PoTokenScope, u64>,
}

type InflightSender = watch::Sender<Option<Result<PoToken, PoTokenError>>>;

/// Caching + single-flight deduplicating wrapper around any [`PoTokenProvider`].
pub struct CachingPoTokenProvider<P: PoTokenProvider + 'static> {
    inner: Arc<P>,
    state: Arc<RwLock<CacheState>>,
    inflight: Mutex<HashMap<CacheKey, InflightSender>>,
    clock: ClockFn,
}

impl<P: PoTokenProvider + 'static> CachingPoTokenProvider<P> {
    pub fn new(inner: P) -> Self {
        Self::with_clock(inner, Arc::new(unix_secs))
    }

    pub fn with_clock(inner: P, clock: ClockFn) -> Self {
        Self {
            inner: Arc::new(inner),
            state: Arc::new(RwLock::new(CacheState::default())),
            inflight: Mutex::new(HashMap::new()),
            clock,
        }
    }

    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }

    pub fn inner(&self) -> &P {
        &self.inner
    }

    /// Resets negative-cache backoff entries (e.g., on network change or auth state change).
    pub fn reset_negative_cache(&self) {
        self.state
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .negative
            .clear();
    }

    /// Coalesced 403 invalidation with a 10s debounce window per [`PoTokenScope`].
    pub fn invalidate_debounced(&self, scope: PoTokenScope) -> bool {
        let now = (self.clock)();
        {
            let mut guard = self.state.write().unwrap_or_else(|p| p.into_inner());
            if let Some(&last) = guard.last_invalidated.get(&scope)
                && now < last.saturating_add(INVALIDATE_DEBOUNCE_SECS)
            {
                return false;
            }
            guard.last_invalidated.insert(scope, now);
        }
        self.invalidate(scope);
        true
    }

    fn lookup_fresh_or_schedule_refresh(
        &self,
        key: &CacheKey,
        req: &PoTokenRequest,
    ) -> Option<PoToken> {
        let now = (self.clock)();
        let mut should_refresh = false;
        let token = {
            let mut guard = self.state.write().unwrap_or_else(|p| p.into_inner());
            let entry = guard.entries.get_mut(key)?;
            if entry.token.expires_at_unix_secs <= now.saturating_add(CACHE_EXPIRY_SKEW_SECS) {
                return None;
            }
            let total_ttl = entry
                .token
                .expires_at_unix_secs
                .saturating_sub(entry.minted_at_unix_secs);
            let elapsed = now.saturating_sub(entry.minted_at_unix_secs);
            if total_ttl > 0
                && elapsed.saturating_mul(5) >= total_ttl.saturating_mul(4)
                && !entry.refreshing
            {
                entry.refreshing = true;
                should_refresh = true;
            }
            entry.token.clone()
        };

        if should_refresh {
            let inner = Arc::clone(&self.inner);
            let state = Arc::clone(&self.state);
            let clock = Arc::clone(&self.clock);
            let key_owned = key.clone();
            let req_owned = req.clone();
            tokio::spawn(async move {
                match inner.mint_token(&req_owned).await {
                    Ok(fresh) => {
                        let minted_at = (clock)();
                        let mut guard = state.write().unwrap_or_else(|p| p.into_inner());
                        guard.negative.remove(&key_owned);
                        guard.entries.insert(
                            key_owned,
                            CacheEntry {
                                token: fresh,
                                minted_at_unix_secs: minted_at,
                                refreshing: false,
                            },
                        );
                    }
                    Err(_) => {
                        let mut guard = state.write().unwrap_or_else(|p| p.into_inner());
                        if let Some(entry) = guard.entries.get_mut(&key_owned) {
                            entry.refreshing = false;
                        }
                    }
                }
            });
        }

        Some(token)
    }

    fn check_negative_backoff(&self, key: &CacheKey) -> Option<PoTokenError> {
        let now = (self.clock)();
        let guard = self.state.read().unwrap_or_else(|p| p.into_inner());
        let neg = guard.negative.get(key)?;
        if now < neg.failed_at_unix_secs.saturating_add(neg.backoff_secs) {
            let remaining = neg
                .failed_at_unix_secs
                .saturating_add(neg.backoff_secs)
                .saturating_sub(now);
            Some(PoTokenError::Unavailable(format!(
                "negative-cache backoff active ({remaining}s remaining)"
            )))
        } else {
            None
        }
    }
}

#[async_trait]
impl<P: PoTokenProvider + 'static> PoTokenProvider for CachingPoTokenProvider<P> {
    async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        let key = CacheKey::from_request(req);
        if let Some(cached) = self.lookup_fresh_or_schedule_refresh(&key, req) {
            return Ok(cached);
        }
        if let Some(backoff_err) = self.check_negative_backoff(&key) {
            return Err(backoff_err);
        }

        // Per-key single-flight via tokio::sync::watch
        let (leader_tx, follower_rx) = {
            let mut inflight = self.inflight.lock().await;
            if let Some(cached) = self.lookup_fresh_or_schedule_refresh(&key, req) {
                return Ok(cached);
            }
            if let Some(existing_tx) = inflight.get(&key) {
                (None, Some(existing_tx.subscribe()))
            } else {
                let (tx, _rx) = watch::channel(None);
                inflight.insert(key.clone(), tx.clone());
                (Some(tx), None)
            }
        };

        if let Some(mut rx) = follower_rx {
            let _ = rx.wait_for(|v| v.is_some()).await;
            return rx.borrow().as_ref().cloned().unwrap_or_else(|| {
                Err(PoTokenError::Unavailable("single-flight cancelled".into()))
            });
        }

        let tx = leader_tx.expect("leader sender");
        let res = self.inner.mint_token(req).await;
        let now = (self.clock)();

        {
            let mut guard = self.state.write().unwrap_or_else(|p| p.into_inner());
            match &res {
                Ok(minted) => {
                    guard.negative.remove(&key);
                    guard.entries.insert(
                        key.clone(),
                        CacheEntry {
                            token: minted.clone(),
                            minted_at_unix_secs: now,
                            refreshing: false,
                        },
                    );
                }
                Err(_) => {
                    let next_backoff = guard
                        .negative
                        .get(&key)
                        .map(|prev| {
                            prev.backoff_secs
                                .saturating_mul(2)
                                .min(MAX_NEGATIVE_BACKOFF_SECS)
                        })
                        .unwrap_or(INITIAL_NEGATIVE_BACKOFF_SECS);
                    guard.negative.insert(
                        key.clone(),
                        NegativeEntry {
                            failed_at_unix_secs: now,
                            backoff_secs: next_backoff,
                        },
                    );
                }
            }
        }

        {
            let mut inflight = self.inflight.lock().await;
            inflight.remove(&key);
        }
        let _ = tx.send(Some(res.clone()));
        res
    }

    fn invalidate(&self, scope: PoTokenScope) {
        let mut guard = self.state.write().unwrap_or_else(|p| p.into_inner());
        guard.entries.retain(|k, _| k.scope != scope);
        guard.negative.retain(|k, _| k.scope != scope);
        drop(guard);
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

        assert_eq!(
            CacheKey::from_request(&gvs_a),
            CacheKey::from_request(&gvs_b)
        );
        assert_ne!(
            CacheKey::from_request(&gvs_a),
            CacheKey::from_request(&gvs_other)
        );
        assert_ne!(
            CacheKey::from_request(&subs_v1),
            CacheKey::from_request(&subs_v2)
        );
    }
}
