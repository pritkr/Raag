//! YouTube Music account profile and Premium tier auto-detection (`ST-1B`).
//!
//! Calls `WEB_REMIX` `account/account_menu` using the active authenticated session,
//! extracts display name, channel handle, avatar URL, and Premium status, and
//! persists updates into `AuthSession` + `InnerTubeClient::set_premium_account()`.
//! Transient network/5xx errors preserve the previously persisted `is_premium` state.

use resonance_contract::{AccountProfile, CoreError, CoreResult};
use serde_json::{Value, json};

use crate::innertube::InnerTubeClient;
use crate::innertube::auth::redact_secrets;
use crate::unix_secs;

pub struct AccountProfileProbe;

impl AccountProfileProbe {
    /// Probes `account/account_menu` on `WEB_REMIX`, updates `AuthSession` & `is_premium_account`,
    /// and returns the detected [`AccountProfile`].
    ///
    /// On transient network/HTTP errors, the previous `session.is_premium` and
    /// `session.account_name` are preserved untouched.
    pub async fn probe_and_update(client: &InnerTubeClient) -> CoreResult<AccountProfile> {
        let payload = json!({
            "deviceTheme": "DEVICE_THEME_SUPPORTED"
        });
        let raw = client
            .post_authenticated_web_remix("account/account_menu", payload)
            .await
            .map_err(|e| {
                // Redact any cookie/token fragments from error diagnostics; never clobber session.
                match e {
                    CoreError::Provider(pe) => CoreError::Provider(
                        resonance_contract::ProviderError::Network(redact_secrets(&pe.to_string())),
                    ),
                    other => other,
                }
            })?;

        let profile = Self::parse_account_menu(&raw, unix_secs())?;

        if let Some(mut session) = client.current_session() {
            if !profile.account_name.is_empty() {
                session.account_name = Some(profile.account_name.clone());
            }
            session.is_premium = profile.is_premium;
            client.sign_in(session)?;
            client.set_premium_account(profile.is_premium);
        }

        Ok(profile)
    }

    /// Secondary ground-truth confirmation during playback: if any `WEB_REMIX` `/player`
    /// response returns audio itag `774` (256 kbps Opus) or `141` (256 kbps AAC),
    /// upgrades `is_premium` to `true` and persists the updated `AuthSession`.
    pub fn confirm_premium_from_itags(client: &InnerTubeClient, itags: &[u32]) -> bool {
        if !itags.iter().any(|&itag| itag == 774 || itag == 141) {
            return false;
        }
        if let Some(mut session) = client.current_session() {
            if !session.is_premium {
                session.is_premium = true;
                let _ = client.sign_in(session);
            }
            client.set_premium_account(true);
            return true;
        }
        false
    }

    /// Defensive multi-signal parser for `account/account_menu` responses.
    pub fn parse_account_menu(
        raw: &Value,
        detected_at_unix_secs: u64,
    ) -> CoreResult<AccountProfile> {
        let header = find_object_by_key(raw, "activeAccountHeaderRenderer");

        let account_name = header
            .and_then(|h| extract_text(h.get("accountName")?))
            .unwrap_or_default();

        let channel_handle = header
            .and_then(|h| h.get("channelHandle"))
            .and_then(extract_text)
            .filter(|s| !s.is_empty());

        let avatar_url = header
            .and_then(|h| h.get("accountPhoto"))
            .and_then(|p| p.get("thumbnails"))
            .and_then(Value::as_array)
            .and_then(|arr| arr.last())
            .and_then(|t| t.get("url"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        let is_premium = detect_premium_signals(raw);

        Ok(AccountProfile {
            account_name,
            channel_handle,
            avatar_url,
            is_premium,
            detected_at_unix_secs,
        })
    }
}

fn detect_premium_signals(raw: &Value) -> bool {
    let mut has_paid_membership = false;
    let mut has_upsell_get_premium = false;

    let mut sections = Vec::new();
    collect_objects_by_key(raw, "multiPageMenuSectionRenderer", &mut sections);

    for section in sections {
        let Some(items) = section.get("items").and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let link = item.get("compactLinkRenderer").unwrap_or(item);
            let title = link
                .get("title")
                .and_then(extract_text)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let icon_type = link
                .get("icon")
                .and_then(|i| i.get("iconType"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let browse_id = link
                .get("navigationEndpoint")
                .and_then(|n| n.get("browseEndpoint"))
                .and_then(|b| b.get("browseId"))
                .and_then(Value::as_str)
                .unwrap_or_default();

            let text_blob = format!("{title} {icon_type} {browse_id}");
            if title.contains("get music premium")
                || title.contains("try youtube music premium")
                || text_blob.contains("music_premium")
            {
                has_upsell_get_premium = true;
            }
            if title.contains("paid memberships")
                || title.contains("manage membership")
                || icon_type.contains("manage_membership")
                || browse_id == "SPunlimited"
            {
                // `SPunlimited` with "Get Music Premium" is an upsell; only count as paid
                // when it is not an upsell title.
                if !title.contains("get music premium") {
                    has_paid_membership = true;
                }
            }
        }
    }

    has_paid_membership && !has_upsell_get_premium
}

fn extract_text(val: &Value) -> Option<String> {
    if let Some(simple) = val.get("simpleText").and_then(Value::as_str) {
        return Some(simple.to_owned());
    }
    if let Some(runs) = val.get("runs").and_then(Value::as_array) {
        let joined: String = runs
            .iter()
            .filter_map(|r| r.get("text").and_then(Value::as_str))
            .collect();
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    None
}

fn find_object_by_key<'a>(val: &'a Value, target_key: &str) -> Option<&'a Value> {
    match val {
        Value::Object(map) => {
            if let Some(found) = map.get(target_key) {
                return Some(found);
            }
            for v in map.values() {
                if let Some(found) = find_object_by_key(v, target_key) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(arr) => {
            for item in arr {
                if let Some(found) = find_object_by_key(item, target_key) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn collect_objects_by_key<'a>(val: &'a Value, target_key: &str, out: &mut Vec<&'a Value>) {
    match val {
        Value::Object(map) => {
            if let Some(found) = map.get(target_key) {
                out.push(found);
            }
            for v in map.values() {
                collect_objects_by_key(v, target_key, out);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                collect_objects_by_key(item, target_key, out);
            }
        }
        _ => {}
    }
}
