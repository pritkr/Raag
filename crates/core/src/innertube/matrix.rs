//! Data-driven InnerTube client viability matrix, PO-token policy engine,
//! and adaptive per-client Circuit Breaker (`Closed -> Open -> HalfOpen`).
//!
//! Replaces hard-coded client chains with a remote-updatable `client_matrix.json`
//! refreshed by `client_probe`, combined with rolling HTTP 403/429 & latency telemetry
//! so unhealthy clients trip into cooldown instead of causing repeated stalls on every skip.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::Path;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use resonance_contract::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};

use crate::innertube::clients::{AuthMode, ClientConfig, ClientKind};

/// Embedded baseline matrix shipped with the app; overridden at runtime whenever
/// a fresher `client_matrix.json` exists in `<data_dir>/config/client_matrix.json`.
pub const EMBEDDED_CLIENT_MATRIX_JSON: &str = include_str!("../../../../config/client_matrix.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoTokenPolicy {
    NotRequired,
    #[serde(rename = "required")]
    Required,
    /// Web clients (`WEB_REMIX`) require a GVS PO token for free/anonymous sessions,
    /// while YouTube Music Premium subscribers bypass the GVS PO token requirement.
    RequiredForWebFreeBypassedByPremium,
    /// iOS clients require an `iOSGuard` PoToken.
    RequiredIosGuard,
    Optional,
}

impl PoTokenPolicy {
    pub fn requires_gvs_po_token(self, tier: AccountTier) -> bool {
        match self {
            Self::Required => true,
            Self::RequiredForWebFreeBypassedByPremium => !tier.is_premium(),
            Self::NotRequired | Self::Optional | Self::RequiredIosGuard => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountTier {
    Anonymous,
    FreeSignedIn,
    PremiumSignedIn,
}

impl AccountTier {
    pub fn is_signed_in(self) -> bool {
        !matches!(self, Self::Anonymous)
    }

    pub fn is_premium(self) -> bool {
        matches!(self, Self::PremiumSignedIn)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeVideoSpec {
    pub id: String,
    pub label: String,
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientMatrixEntry {
    pub kind: String,
    pub enabled: bool,
    pub priority_signed_in: u32,
    pub priority_anonymous: u32,
    pub auth_mode: String,
    pub po_token_policy: PoTokenPolicy,
    pub requires_ejs_solver: bool,
    pub probe_pass_cookies_on: bool,
    pub probe_pass_cookies_off: bool,
    #[serde(default)]
    pub notes: String,
}

impl ClientMatrixEntry {
    pub fn client_kind(&self) -> Option<ClientKind> {
        match self.kind.as_str() {
            "web_remix" => Some(ClientKind::WebRemix),
            "vision_os" => Some(ClientKind::VisionOs),
            "android_vr" => Some(ClientKind::AndroidVr),
            "tv_samsung" => Some(ClientKind::TvSamsung),
            "ios" => Some(ClientKind::Ios),
            _ => None,
        }
    }

    /// Evaluates whether this client can legally satisfy a playback request under
    /// the current account tier and PO-token availability.
    pub fn is_eligible(
        &self,
        tier: AccountTier,
        has_gvs_po_token: bool,
        has_ios_guard_po_token: bool,
        has_tv_oauth: bool,
    ) -> bool {
        if !self.enabled {
            return false;
        }
        let Some(kind) = self.client_kind() else {
            return false;
        };
        let cfg = ClientConfig::for_kind(kind);

        // Enforce strict per-client auth isolation
        match cfg.auth_mode {
            AuthMode::TvOAuthOnly if !has_tv_oauth => return false,
            AuthMode::AnonymousOnly if tier.is_signed_in() && !self.probe_pass_cookies_off => {
                return false;
            }
            _ => {}
        }

        // Enforce explicit PO-token policy
        match self.po_token_policy {
            PoTokenPolicy::NotRequired | PoTokenPolicy::Optional => true,
            PoTokenPolicy::Required => has_gvs_po_token,
            PoTokenPolicy::RequiredForWebFreeBypassedByPremium => {
                tier.is_premium() || has_gvs_po_token
            }
            PoTokenPolicy::RequiredIosGuard => has_ios_guard_po_token,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientMatrix {
    pub schema_version: u32,
    pub updated_at_unix_secs: u64,
    pub source: String,
    #[serde(default)]
    pub po_token_policy_notes: String,
    #[serde(default)]
    pub probe_videos: Vec<ProbeVideoSpec>,
    pub clients: Vec<ClientMatrixEntry>,
}

impl ClientMatrix {
    /// Loads the embedded default `client_matrix.json`.
    pub fn embedded_default() -> CoreResult<Self> {
        serde_json::from_str(EMBEDDED_CLIENT_MATRIX_JSON).map_err(|e| {
            CoreError::Provider(resonance_contract::ProviderError::Parse(format!(
                "invalid embedded client_matrix.json: {e}"
            )))
        })
    }

    /// Loads a remote-updated `client_matrix.json` from `override_path` if present,
    /// falling back cleanly to the embedded baseline if missing or corrupted.
    pub fn load_with_override(override_path: &Path) -> CoreResult<Self> {
        if override_path.exists()
            && let Ok(raw) = fs::read_to_string(override_path)
            && let Ok(parsed) = serde_json::from_str::<Self>(&raw)
        {
            return Ok(parsed);
        }
        Self::embedded_default()
    }

    /// Saves a freshly probed matrix atomically (`.tmp` -> rename).
    pub fn save_atomic(&self, path: &Path) -> CoreResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                CoreError::Cache(resonance_contract::CacheError::Io(format!(
                    "create matrix dir: {e}"
                )))
            })?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| {
            CoreError::Cache(resonance_contract::CacheError::Io(format!(
                "serialize client_matrix: {e}"
            )))
        })?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|e| {
            CoreError::Cache(resonance_contract::CacheError::Io(format!(
                "write matrix tmp: {e}"
            )))
        })?;
        fs::rename(&tmp, path).map_err(|e| {
            CoreError::Cache(resonance_contract::CacheError::Io(format!(
                "rename matrix: {e}"
            )))
        })?;
        Ok(())
    }

    /// Looks up the matrix entry for a given [`ClientKind`].
    pub fn entry_for(&self, kind: ClientKind) -> Option<&ClientMatrixEntry> {
        self.clients
            .iter()
            .find(|entry| entry.client_kind() == Some(kind))
    }

    /// Computes the ordered playback resolution chain from the matrix for the given
    /// account tier and PO-token state.
    pub fn playback_chain(
        &self,
        tier: AccountTier,
        has_gvs_po_token: bool,
        has_ios_guard_po_token: bool,
        has_tv_oauth: bool,
    ) -> Vec<&'static ClientConfig> {
        let mut candidates: Vec<(u32, &'static ClientConfig)> = self
            .clients
            .iter()
            .filter(|e| e.is_eligible(tier, has_gvs_po_token, has_ios_guard_po_token, has_tv_oauth))
            .filter_map(|e| {
                let kind = e.client_kind()?;
                let prio = if tier.is_signed_in() {
                    e.priority_signed_in
                } else {
                    e.priority_anonymous
                };
                Some((prio, ClientConfig::for_kind(kind)))
            })
            .collect();
        candidates.sort_by_key(|(prio, _)| *prio);
        candidates.into_iter().map(|(_, cfg)| cfg).collect()
    }
}

// ============================================================================
// Adaptive Client Health Tracker & Circuit Breaker
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone, Copy)]
pub struct CircuitBreakerConfig {
    /// Rolling sample window length per client.
    pub window_size: usize,
    /// Consecutive 403/429 or hard failures required to trip into `Open`.
    pub failure_threshold: u32,
    /// Rolling 403/429 ratio (0.0..=1.0) over at least `failure_threshold` samples to trip `Open`.
    pub max_error_rate: f32,
    /// Duration an `Open` client remains skipped before transitioning to `HalfOpen`.
    pub cooldown: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            window_size: 20,
            failure_threshold: 3,
            max_error_rate: 0.60,
            cooldown: Duration::from_secs(90),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum OutcomeSample {
    Success { latency_ms: u64 },
    Http403 { latency_ms: u64 },
    Http429 { latency_ms: u64 },
    OtherFailure { latency_ms: u64 },
}

/// Snapshot of rolling health telemetry for one InnerTube client.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientHealthSnapshot {
    pub kind: ClientKind,
    pub state: CircuitState,
    pub sample_count: usize,
    pub success_rate: f32,
    pub rate_403: f32,
    pub rate_429: f32,
    pub ewma_latency_ms: f32,
    pub consecutive_failures: u32,
}

#[derive(Debug, Clone, Copy)]
enum BreakerPhase {
    Closed,
    Open {
        opened_at: Instant,
    },
    HalfOpenProbing {
        opened_at: Instant,
        started_at: Instant,
    },
}

struct PerClientHealth {
    samples: VecDeque<OutcomeSample>,
    consecutive_failures: u32,
    ewma_latency_ms: f32,
    phase: BreakerPhase,
}

impl Default for PerClientHealth {
    fn default() -> Self {
        Self {
            samples: VecDeque::with_capacity(24),
            consecutive_failures: 0,
            ewma_latency_ms: 0.0,
            phase: BreakerPhase::Closed,
        }
    }
}

/// Adaptive per-client health tracker & circuit breaker for [`ClientMatrix`].
///
/// Tracks rolling success rate, HTTP 403/429 rates, and EWMA latency, tripping
/// unhealthy clients into `Open` cooldown and recovering via `HalfOpen` probes.
pub struct ClientCircuitBreaker {
    config: CircuitBreakerConfig,
    clients: RwLock<HashMap<ClientKind, PerClientHealth>>,
}

impl Default for ClientCircuitBreaker {
    fn default() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }
}

impl ClientCircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            clients: RwLock::new(HashMap::new()),
        }
    }

    fn phase_to_state(phase: BreakerPhase, cooldown: Duration, now: Instant) -> CircuitState {
        match phase {
            BreakerPhase::Closed => CircuitState::Closed,
            BreakerPhase::Open { opened_at }
                if now.saturating_duration_since(opened_at) >= cooldown =>
            {
                CircuitState::HalfOpen
            }
            BreakerPhase::Open { .. } => CircuitState::Open,
            BreakerPhase::HalfOpenProbing { .. } => CircuitState::HalfOpen,
        }
    }

    pub fn state_at(&self, kind: ClientKind, now: Instant) -> CircuitState {
        let guard = self.clients.read().unwrap();
        let Some(entry) = guard.get(&kind) else {
            return CircuitState::Closed;
        };
        Self::phase_to_state(entry.phase, self.config.cooldown, now)
    }

    pub fn state(&self, kind: ClientKind) -> CircuitState {
        self.state_at(kind, Instant::now())
    }

    /// Returns `true` if the client may be attempted right now (`Closed` or single `HalfOpen` probe).
    pub fn allow_request(&self, kind: ClientKind) -> bool {
        self.allow_request_at(kind, Instant::now())
    }

    pub fn allow_request_at(&self, kind: ClientKind, now: Instant) -> bool {
        let mut guard = self.clients.write().unwrap();
        let entry = guard.entry(kind).or_default();
        match entry.phase {
            BreakerPhase::Closed => true,
            BreakerPhase::Open { opened_at }
                if now.saturating_duration_since(opened_at) >= self.config.cooldown =>
            {
                entry.phase = BreakerPhase::HalfOpenProbing {
                    opened_at,
                    started_at: now,
                };
                true
            }
            BreakerPhase::Open { .. } => false,
            BreakerPhase::HalfOpenProbing {
                opened_at,
                started_at,
            } if now.saturating_duration_since(started_at) >= self.config.cooldown => {
                // Self-heal if a previous probe task was cancelled or panicked before reporting.
                entry.phase = BreakerPhase::HalfOpenProbing {
                    opened_at,
                    started_at: now,
                };
                true
            }
            BreakerPhase::HalfOpenProbing { .. } => false,
        }
    }

    /// Releases an in-flight `HalfOpen` probe slot without tripping or closing the circuit
    /// (used when a probe ends with a non-tripping error or async cancellation).
    pub fn release_probe(&self, kind: ClientKind) {
        let mut guard = self.clients.write().unwrap();
        if let Some(entry) = guard.get_mut(&kind)
            && let BreakerPhase::HalfOpenProbing { opened_at, .. } = entry.phase
        {
            entry.phase = BreakerPhase::Open { opened_at };
        }
    }

    pub fn record_success(&self, kind: ClientKind, latency_ms: u64) {
        let mut guard = self.clients.write().unwrap();
        let entry = guard.entry(kind).or_default();
        Self::push_sample(
            entry,
            OutcomeSample::Success { latency_ms },
            self.config.window_size,
        );
        entry.consecutive_failures = 0;
        entry.phase = BreakerPhase::Closed;
    }

    pub fn record_failure(&self, kind: ClientKind, http_status: Option<u16>, latency_ms: u64) {
        self.record_failure_at(kind, http_status, latency_ms, Instant::now());
    }

    pub fn record_failure_at(
        &self,
        kind: ClientKind,
        http_status: Option<u16>,
        latency_ms: u64,
        now: Instant,
    ) {
        let mut guard = self.clients.write().unwrap();
        let entry = guard.entry(kind).or_default();
        let sample = match http_status {
            Some(403) => OutcomeSample::Http403 { latency_ms },
            Some(429) => OutcomeSample::Http429 { latency_ms },
            _ => OutcomeSample::OtherFailure { latency_ms },
        };
        let was_half_open = matches!(entry.phase, BreakerPhase::HalfOpenProbing { .. });
        Self::push_sample(entry, sample, self.config.window_size);
        entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);

        let err_rate = Self::compute_error_rate(entry);
        let should_open = was_half_open
            || entry.consecutive_failures >= self.config.failure_threshold
            || (entry.samples.len() >= self.config.failure_threshold as usize
                && err_rate >= self.config.max_error_rate);

        if should_open {
            entry.phase = BreakerPhase::Open { opened_at: now };
        }
    }

    pub fn health_snapshot(&self, kind: ClientKind) -> ClientHealthSnapshot {
        let now = Instant::now();
        let guard = self.clients.read().unwrap();
        let Some(entry) = guard.get(&kind) else {
            return ClientHealthSnapshot {
                kind,
                state: CircuitState::Closed,
                sample_count: 0,
                success_rate: 1.0,
                rate_403: 0.0,
                rate_429: 0.0,
                ewma_latency_ms: 0.0,
                consecutive_failures: 0,
            };
        };
        let total = entry.samples.len().max(1) as f32;
        let ok_cnt = entry
            .samples
            .iter()
            .filter(|s| matches!(s, OutcomeSample::Success { .. }))
            .count() as f32;
        let c403 = entry
            .samples
            .iter()
            .filter(|s| matches!(s, OutcomeSample::Http403 { .. }))
            .count() as f32;
        let c429 = entry
            .samples
            .iter()
            .filter(|s| matches!(s, OutcomeSample::Http429 { .. }))
            .count() as f32;
        let state = Self::phase_to_state(entry.phase, self.config.cooldown, now);
        ClientHealthSnapshot {
            kind,
            state,
            sample_count: entry.samples.len(),
            success_rate: if entry.samples.is_empty() {
                1.0
            } else {
                ok_cnt / total
            },
            rate_403: if entry.samples.is_empty() {
                0.0
            } else {
                c403 / total
            },
            rate_429: if entry.samples.is_empty() {
                0.0
            } else {
                c429 / total
            },
            ewma_latency_ms: entry.ewma_latency_ms,
            consecutive_failures: entry.consecutive_failures,
        }
    }

    fn push_sample(entry: &mut PerClientHealth, sample: OutcomeSample, window_size: usize) {
        let latency = match sample {
            OutcomeSample::Success { latency_ms }
            | OutcomeSample::Http403 { latency_ms }
            | OutcomeSample::Http429 { latency_ms }
            | OutcomeSample::OtherFailure { latency_ms } => latency_ms as f32,
        };
        if entry.samples.is_empty() {
            entry.ewma_latency_ms = latency;
        } else {
            entry.ewma_latency_ms = 0.25 * latency + 0.75 * entry.ewma_latency_ms;
        }
        if entry.samples.len() >= window_size.max(1) {
            entry.samples.pop_front();
        }
        entry.samples.push_back(sample);
    }

    fn compute_error_rate(entry: &PerClientHealth) -> f32 {
        if entry.samples.is_empty() {
            return 0.0;
        }
        let bad = entry
            .samples
            .iter()
            .filter(|s| !matches!(s, OutcomeSample::Success { .. }))
            .count();
        bad as f32 / entry.samples.len() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circuit_breaker_trips_on_403_429_and_recovers_after_half_open_cooldown() {
        let cfg = CircuitBreakerConfig {
            window_size: 10,
            failure_threshold: 2,
            max_error_rate: 0.5,
            cooldown: Duration::from_secs(30),
        };
        let cb = ClientCircuitBreaker::new(cfg);
        let t0 = Instant::now();

        assert_eq!(cb.state_at(ClientKind::WebRemix, t0), CircuitState::Closed);
        assert!(cb.allow_request_at(ClientKind::WebRemix, t0));

        cb.record_failure_at(ClientKind::WebRemix, Some(403), 120, t0);
        assert_eq!(cb.state_at(ClientKind::WebRemix, t0), CircuitState::Closed);

        cb.record_failure_at(ClientKind::WebRemix, Some(429), 200, t0);
        assert_eq!(cb.state_at(ClientKind::WebRemix, t0), CircuitState::Open);
        assert!(!cb.allow_request_at(ClientKind::WebRemix, t0 + Duration::from_secs(10)));

        // After cooldown -> HalfOpen allows single probe
        let t_after = t0 + Duration::from_secs(31);
        assert_eq!(
            cb.state_at(ClientKind::WebRemix, t_after),
            CircuitState::HalfOpen
        );
        assert!(cb.allow_request_at(ClientKind::WebRemix, t_after));
        assert!(!cb.allow_request_at(ClientKind::WebRemix, t_after));

        // Probe succeeds -> Closed
        cb.record_success(ClientKind::WebRemix, 80);
        assert_eq!(
            cb.state_at(ClientKind::WebRemix, t_after),
            CircuitState::Closed
        );
        let snap = cb.health_snapshot(ClientKind::WebRemix);
        assert!(snap.rate_403 > 0.0);
        assert!(snap.rate_429 > 0.0);
        assert!(snap.ewma_latency_ms > 0.0);
    }
}
