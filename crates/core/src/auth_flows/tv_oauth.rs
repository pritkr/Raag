//! RFC 8628 YouTube TV OAuth Device-Code Flow & single-flight token refresh (`H3.1`).

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use resonance_contract::{AuthError, CoreError, CoreResult, ProviderError};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock, watch};

use crate::innertube::auth::redact_secrets;
use crate::unix_secs;

pub const GOOGLE_DEVICE_CODE_URL: &str = "https://oauth2.googleapis.com/device/code";
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const YOUTUBE_OAUTH_SCOPE: &str = "https://www.googleapis.com/auth/youtube";
pub const REFRESH_LEEWAY_SECS: u64 = 120;

#[derive(Debug, Clone)]
pub struct TvOAuthConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub device_code_url: String,
    pub token_url: String,
    pub scope: String,
}

impl TvOAuthConfig {
    pub fn from_env_or_default() -> Self {
        Self {
            client_id: std::env::var("RESONANCE_OAUTH_CLIENT_ID").unwrap_or_else(|_| {
                "861556708454-d6dlm3lh05idd8npek18k6be8ba3oc68.apps.googleusercontent.com"
                    .to_owned()
            }),
            client_secret: std::env::var("RESONANCE_OAUTH_CLIENT_SECRET").ok(),
            device_code_url: GOOGLE_DEVICE_CODE_URL.to_owned(),
            token_url: GOOGLE_TOKEN_URL.to_owned(),
            scope: YOUTUBE_OAUTH_SCOPE.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    #[serde(alias = "verification_uri")]
    pub verification_url: String,
    pub expires_in: u64,
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_interval() -> u64 {
    5
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TvOAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_unix_secs: u64,
    pub token_type: String,
}

#[derive(Debug, Deserialize)]
struct TokenSuccessBody {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    #[serde(default = "default_bearer")]
    token_type: String,
}

fn default_bearer() -> String {
    "Bearer".to_owned()
}

#[derive(Debug, Deserialize)]
struct TokenErrorBody {
    error: String,
}

/// Redacts OAuth access/refresh/device tokens in addition to standard Resonance secrets.
pub fn redact_oauth_secrets(input: &str) -> String {
    let base = redact_secrets(input);
    let mut out = String::with_capacity(base.len());
    for line in base.split_inclusive('\n') {
        let lower = line.to_ascii_lowercase();
        if lower.contains("access_token")
            || lower.contains("refresh_token")
            || lower.contains("device_code")
            || lower.contains("ya29.")
        {
            let has_nl = line.ends_with('\n');
            out.push_str("[REDACTED_OAUTH_LINE]");
            if has_nl {
                out.push('\n');
            }
        } else {
            out.push_str(line);
        }
    }
    out
}

#[derive(Clone)]
pub struct TvOAuthClient {
    http: Client,
    config: TvOAuthConfig,
}

impl TvOAuthClient {
    pub fn new(config: TvOAuthConfig) -> CoreResult<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| {
                CoreError::Provider(ProviderError::Network(redact_oauth_secrets(&e.to_string())))
            })?;
        Ok(Self { http, config })
    }

    pub async fn request_device_code(&self) -> CoreResult<DeviceCodeResponse> {
        let params = [
            ("client_id", self.config.client_id.as_str()),
            ("scope", self.config.scope.as_str()),
        ];
        let resp = self
            .http
            .post(&self.config.device_code_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| {
                CoreError::Provider(ProviderError::Network(redact_oauth_secrets(&e.to_string())))
            })?;

        if !resp.status().is_success() {
            return Err(CoreError::Provider(ProviderError::Http {
                status: resp.status().as_u16(),
                message: "device_code request failed".to_owned(),
            }));
        }

        resp.json::<DeviceCodeResponse>().await.map_err(|e| {
            CoreError::Provider(ProviderError::Parse(redact_oauth_secrets(&e.to_string())))
        })
    }

    pub async fn poll_for_token(
        &self,
        device_code: &str,
        initial_interval_secs: u64,
        expires_in_secs: u64,
        mut cancel_rx: watch::Receiver<bool>,
    ) -> CoreResult<TvOAuthTokens> {
        let mut interval_secs = initial_interval_secs.max(1);
        let deadline = unix_secs().saturating_add(expires_in_secs);

        loop {
            if *cancel_rx.borrow() {
                return Err(CoreError::Auth(AuthError::NotSignedIn));
            }
            if unix_secs() >= deadline {
                return Err(CoreError::Auth(AuthError::Revoked));
            }

            let mut params = vec![
                ("client_id", self.config.client_id.as_str()),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ];
            if let Some(secret) = self.config.client_secret.as_deref() {
                params.push(("client_secret", secret));
            }

            let resp = self
                .http
                .post(&self.config.token_url)
                .form(&params)
                .send()
                .await
                .map_err(|e| {
                    CoreError::Provider(ProviderError::Network(redact_oauth_secrets(
                        &e.to_string(),
                    )))
                })?;

            let status = resp.status();
            let body_bytes = resp.bytes().await.map_err(|e| {
                CoreError::Provider(ProviderError::Network(redact_oauth_secrets(&e.to_string())))
            })?;

            if status.is_success() {
                let ok = serde_json::from_slice::<TokenSuccessBody>(&body_bytes).map_err(|e| {
                    CoreError::Provider(ProviderError::Parse(redact_oauth_secrets(&e.to_string())))
                })?;
                return Ok(TvOAuthTokens {
                    access_token: ok.access_token,
                    refresh_token: ok.refresh_token.unwrap_or_default(),
                    expires_at_unix_secs: unix_secs().saturating_add(ok.expires_in),
                    token_type: ok.token_type,
                });
            }

            if let Ok(err_body) = serde_json::from_slice::<TokenErrorBody>(&body_bytes) {
                match err_body.error.as_str() {
                    "authorization_pending" => {}
                    "slow_down" => {
                        interval_secs = interval_secs.saturating_add(5);
                    }
                    "expired_token" => return Err(CoreError::Auth(AuthError::Revoked)),
                    "access_denied" => return Err(CoreError::Auth(AuthError::NotSignedIn)),
                    other => {
                        return Err(CoreError::Provider(ProviderError::Http {
                            status: status.as_u16(),
                            message: redact_oauth_secrets(other),
                        }));
                    }
                }
            } else {
                return Err(CoreError::Provider(ProviderError::Http {
                    status: status.as_u16(),
                    message: "oauth token poll error".to_owned(),
                }));
            }

            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {}
                changed = cancel_rx.changed() => {
                    if changed.is_ok() && *cancel_rx.borrow() {
                        return Err(CoreError::Auth(AuthError::NotSignedIn));
                    }
                }
            }
        }
    }

    pub async fn refresh_access_token(&self, refresh_token: &str) -> CoreResult<TvOAuthTokens> {
        let mut params = vec![
            ("client_id", self.config.client_id.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ];
        if let Some(secret) = self.config.client_secret.as_deref() {
            params.push(("client_secret", secret));
        }

        let resp = self
            .http
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| {
                CoreError::Provider(ProviderError::Network(redact_oauth_secrets(&e.to_string())))
            })?;

        if !resp.status().is_success() {
            return Err(CoreError::Auth(AuthError::Revoked));
        }

        let ok = resp.json::<TokenSuccessBody>().await.map_err(|e| {
            CoreError::Provider(ProviderError::Parse(redact_oauth_secrets(&e.to_string())))
        })?;
        Ok(TvOAuthTokens {
            access_token: ok.access_token,
            refresh_token: ok.refresh_token.unwrap_or_else(|| refresh_token.to_owned()),
            expires_at_unix_secs: unix_secs().saturating_add(ok.expires_in),
            token_type: ok.token_type,
        })
    }
}

/// Single-flight TV OAuth token manager:
/// proactively refreshes when `now + 120s >= expires_at_unix_secs` under a `Mutex<()>`
/// so concurrent API calls share one refresh request.
pub struct TvOAuthTokenManager {
    client: TvOAuthClient,
    tokens: Arc<RwLock<TvOAuthTokens>>,
    refresh_lock: Mutex<()>,
}

impl TvOAuthTokenManager {
    pub fn new(client: TvOAuthClient, initial: TvOAuthTokens) -> Self {
        Self {
            client,
            tokens: Arc::new(RwLock::new(initial)),
            refresh_lock: Mutex::new(()),
        }
    }

    pub async fn current_tokens(&self) -> TvOAuthTokens {
        self.tokens.read().await.clone()
    }

    pub async fn get_valid_access_token(&self) -> CoreResult<String> {
        {
            let guard = self.tokens.read().await;
            if unix_secs().saturating_add(REFRESH_LEEWAY_SECS) < guard.expires_at_unix_secs {
                return Ok(guard.access_token.clone());
            }
        }

        let _single_flight = self.refresh_lock.lock().await;
        let refresh_token = {
            let guard = self.tokens.read().await;
            if unix_secs().saturating_add(REFRESH_LEEWAY_SECS) < guard.expires_at_unix_secs {
                return Ok(guard.access_token.clone());
            }
            guard.refresh_token.clone()
        };

        let refreshed = self.client.refresh_access_token(&refresh_token).await?;
        let access = refreshed.access_token.clone();
        *self.tokens.write().await = refreshed;
        Ok(access)
    }
}
