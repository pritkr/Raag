//! Update channel validation, semver anti-downgrade guard, and HTTPS URL allowlist.

use resonance_contract::UpdateChannel;

/// Parses a semver string like `"v0.2.0"` or `"0.2.0-beta.1"` into `(major, minor, patch, is_prerelease, pre_num)`.
pub fn parse_semver_tuple(raw: &str) -> Option<(u64, u64, u64, bool, String)> {
    let trimmed = raw.trim().strip_prefix('v').unwrap_or(raw.trim());
    let (core, pre) = match trimmed.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (trimmed, None),
    };
    let mut parts = core.split('.');
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next()?.parse().ok()?;
    let patch: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((
        major,
        minor,
        patch,
        pre.is_some(),
        pre.unwrap_or_default().to_owned(),
    ))
}

/// Anti-downgrade guard: returns `true` strictly when `candidate` is newer than `current`.
pub fn is_newer_semver(candidate: &str, current: &str) -> bool {
    let Some((cmaj, cmin, cpat, cpre, cpre_str)) = parse_semver_tuple(candidate) else {
        return false;
    };
    let Some((maj, min, pat, pre, pre_str)) = parse_semver_tuple(current) else {
        return false;
    };
    match (cmaj, cmin, cpat).cmp(&(maj, min, pat)) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => {
            // Stable release (cpre == false) is newer than prerelease (pre == true) of same X.Y.Z.
            match (cpre, pre) {
                (false, true) => true,
                (true, false) => false,
                (false, false) => false,
                (true, true) => cpre_str > pre_str,
            }
        }
    }
}

/// Returns the NSH channel name declared in `nsh.json` (`None` for Stable, `Some("beta")` for Beta).
pub fn validate_declared_channel(channel: UpdateChannel) -> Option<&'static str> {
    channel.nsh_channel_name()
}

/// Enforces HTTPS + GitHub release origin allowlist before calling `open_url`.
pub fn is_allowed_release_url(raw_url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(raw_url.trim()) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    matches!(
        host,
        "github.com" | "objects.githubusercontent.com" | "release-assets.githubusercontent.com"
    )
}
