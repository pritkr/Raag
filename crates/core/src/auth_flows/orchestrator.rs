//! Single unified `AuthSignInOrchestrator` funneling Embedded WebView,
//! TV OAuth Device-Code, and Manual Cookie Header sign-in into `AuthSession` (`H3.1`).

use resonance_contract::{AuthSession, AuthStore, CoreError, CoreResult};

use crate::auth_flows::cookie_extract::build_auth_session_from_cookies;
use crate::auth_flows::tv_oauth::TvOAuthTokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInMethod {
    EmbeddedWebView,
    TvOAuthDeviceCode,
    ManualCookieHeader,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthSignInOutcome {
    pub method: SignInMethod,
    pub session: Option<AuthSession>,
    pub tv_tokens: Option<TvOAuthTokens>,
}

pub struct AuthSignInOrchestrator;

impl AuthSignInOrchestrator {
    /// Validates captured WebView or pasted cookies (including `HttpOnly` session check),
    /// persists `AuthSession` into `AuthStore` (OS Keychain / Keystore), and returns the outcome.
    pub fn complete_cookie_sign_in(
        store: &dyn AuthStore,
        method: SignInMethod,
        cookie_header: &str,
        auth_user: u32,
    ) -> CoreResult<AuthSignInOutcome> {
        let session =
            build_auth_session_from_cookies(cookie_header, auth_user).map_err(CoreError::Auth)?;
        store.save_session(&session).map_err(CoreError::Auth)?;
        Ok(AuthSignInOutcome {
            method,
            session: Some(session),
            tv_tokens: None,
        })
    }

    /// Wraps completed RFC 8628 TV OAuth tokens into a unified [`AuthSignInOutcome`].
    pub fn complete_tv_oauth_sign_in(tokens: TvOAuthTokens) -> AuthSignInOutcome {
        AuthSignInOutcome {
            method: SignInMethod::TvOAuthDeviceCode,
            session: None,
            tv_tokens: Some(tokens),
        }
    }
}
