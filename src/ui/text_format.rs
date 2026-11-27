//! Shared microcopy, pluralization, typographic quotation, and recoverable error helpers.

/// Formats a grammatically correct singular or plural count (`"1 song"` vs `"4 songs"`).
pub fn pluralize(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}

/// Wraps a user-supplied term in typographic curly quotation marks (`“…”`).
pub fn quote_term(term: &str) -> String {
    format!("\u{201C}{}\u{201D}", term.trim())
}

/// Redacts sensitive query strings (`?token=...`, `?sig=...`) from URLs before showing error text.
pub fn redact_query_params(raw: &str) -> String {
    raw.split_whitespace()
        .map(|tok| {
            if (tok.starts_with("http://") || tok.starts_with("https://"))
                && let Some((base, _)) = tok.split_once('?')
            {
                format!("{base}?[redacted]")
            } else {
                tok.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Formats a user-facing error message with plain cause and concrete next step (no raw stack/JSON dumps).
pub fn format_recoverable_error(context: &str, raw_err: &str, next_step: &str) -> String {
    let clean = redact_query_params(raw_err.trim());
    let summary = if clean.is_empty()
        || clean.starts_with('{')
        || clean.starts_with('[')
        || clean.len() > 160
    {
        format!("{context}.")
    } else {
        let trimmed = clean.trim_end_matches('.');
        format!("{context}: {trimmed}.")
    };
    let step = next_step.trim().trim_end_matches('.');
    if step.is_empty() {
        summary
    } else {
        format!("{summary} {step}.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pluralizes_singular_and_plural_counts() {
        assert_eq!(pluralize(0, "song", "songs"), "0 songs");
        assert_eq!(pluralize(1, "song", "songs"), "1 song");
        assert_eq!(pluralize(4, "track", "tracks"), "4 tracks");
        assert_eq!(pluralize(1, "result", "results"), "1 result");
    }

    #[test]
    fn quotes_terms_with_typographic_marks() {
        assert_eq!(quote_term("  Lo-Fi Beats "), "\u{201C}Lo-Fi Beats\u{201D}");
    }

    #[test]
    fn redacts_url_query_parameters() {
        let input = "Fetch failed for https://music.example.com/stream?token=secret&sig=123";
        assert_eq!(
            redact_query_params(input),
            "Fetch failed for https://music.example.com/stream?[redacted]"
        );
    }

    #[test]
    fn formats_recoverable_errors_without_raw_json() {
        assert_eq!(
            format_recoverable_error(
                "Could not connect to cast device",
                r#"{"error":"ECONNREFUSED"}"#,
                "Check that the device is on the same Wi-Fi network and try again"
            ),
            "Could not connect to cast device. Check that the device is on the same Wi-Fi network and try again."
        );
        assert_eq!(
            format_recoverable_error(
                "Could not check for updates",
                "DNS lookup timed out",
                "Check your connection and retry"
            ),
            "Could not check for updates: DNS lookup timed out. Check your connection and retry."
        );
    }
}
