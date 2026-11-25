//! Unified Desktop & TV OAuth Sign-In flows (`H3.1`).

pub mod cookie_extract;
pub mod orchestrator;
pub mod tv_oauth;

pub use cookie_extract::{ExtractedCookies, build_auth_session_from_cookies, validate_cookie_jar};
pub use orchestrator::{AuthSignInOrchestrator, AuthSignInOutcome, SignInMethod};
pub use tv_oauth::{
    DeviceCodeResponse, TvOAuthClient, TvOAuthConfig, TvOAuthTokenManager, TvOAuthTokens,
    redact_oauth_secrets,
};
