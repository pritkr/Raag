//! Stage 2 & Stage 3 Verification Suite:
//! - LoopbackProxy monotonic generation-ID cancellation (409 on stale ?gen=<n>)
//! - Strict secret redaction (SAPISID, Authorization, Cookie, signed googlevideo URLs)
//! - Off-UI-thread ambient artwork palette extraction + WCAG AA contrast + track_id caching
//! - Search filters (All / Songs / Albums) + Desktop right-hand Queue/Lyrics side panel
//! - Performance rule: ZERO backdrop blur inside virtual_list rows

use std::fs;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use nsh_engine::{EngineConfig, Points, Root, Size};
use nsh_testing::Headless;
use resonance_contract::{
    AudioCodec, CoreResult, HomeSection, LyricsDocument, MusicProvider, Playlist, SearchResults,
    StreamInfo, Track,
};
use resonance_core::cache::{ChunkCache, DEFAULT_MAX_AUDIO_CACHE_BYTES, SqliteCacheStore};
use resonance_core::innertube::auth::redact_secrets;
use resonance_core::palette::{extract_and_cache_from_rgb, palette_for_track};
use resonance_core::proxy::LoopbackProxy;
use tempfile::tempdir;

struct DummyProvider;

#[async_trait]
impl MusicProvider for DummyProvider {
    async fn home(&self) -> CoreResult<Vec<HomeSection>> {
        Ok(Vec::new())
    }
    async fn search(&self, query: &str) -> CoreResult<SearchResults> {
        Ok(SearchResults {
            query: query.to_owned(),
            tracks: Vec::new(),
            albums: Vec::new(),
            continuation: None,
        })
    }
    async fn browse_collection(&self, _id: &str) -> CoreResult<Playlist> {
        Ok(Playlist {
            id: "dummy".into(),
            title: "Dummy".into(),
            subtitle: None,
            artwork_url: None,
            tracks: Vec::new(),
        })
    }
    async fn liked_songs(&self) -> CoreResult<Vec<Track>> {
        Ok(Vec::new())
    }
    async fn autoplay_next(&self, _video_id: &str) -> CoreResult<Vec<Track>> {
        Ok(Vec::new())
    }
    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo> {
        Ok(StreamInfo {
            video_id: video_id.to_owned(),
            url: "http://127.0.0.1:9/unused".into(),
            itag: 251,
            mime_type: "audio/webm; codecs=\"opus\"".into(),
            codec: AudioCodec::Opus,
            bitrate_kbps: 160,
            content_length: 1024,
            duration_ms: 1000,
            loudness_db: None,
            resolved_by_client: "VISIONOS".into(),
            expires_at_unix_secs: u64::MAX,
        })
    }
    async fn ytm_lyrics(&self, _video_id: &str) -> CoreResult<Option<LyricsDocument>> {
        Ok(None)
    }
}

#[tokio::test]
async fn proxy_monotonic_generation_cancels_stale_requests_with_409() {
    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(&dir.path().join("gen.db")).unwrap());
    let chunks = Arc::new(
        ChunkCache::new(
            &dir.path().join("chunks"),
            cache,
            DEFAULT_MAX_AUDIO_CACHE_BYTES,
        )
        .unwrap(),
    );
    let proxy = LoopbackProxy::start(Arc::new(DummyProvider), chunks)
        .await
        .unwrap();

    let gen1 = proxy.advance_generation();
    let stale_url = proxy.stream_url_with_gen("dQw4w9WgXcQ", 251, gen1);
    let gen2 = proxy.advance_generation();
    assert!(gen2 > gen1, "generation ID must advance monotonically");

    let resp = reqwest::Client::new()
        .get(&stale_url)
        .send()
        .await
        .expect("proxy answers stale generation");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::CONFLICT,
        "stale track skip generation must be immediately aborted with 409 Conflict"
    );
}

#[test]
fn secret_redaction_scrubs_cookies_sapisid_auth_and_signed_urls() {
    let raw = "Authorization: SAPISIDHASH 1700000000_deadbeefcafebabe\n\
               Cookie: SAPISID=secret123/abc; __Secure-3PAPISID=secret456; HSID=foo\n\
               URL: https://rr1---sn-abc.googlevideo.com/videoplayback?expire=1700&sig=TOPSECRET&n=abc123";
    let clean = redact_secrets(raw);
    assert!(!clean.contains("secret123"), "must redact SAPISID value");
    assert!(!clean.contains("secret456"), "must redact 3PAPISID value");
    assert!(!clean.contains("deadbeef"), "must redact SAPISIDHASH");
    assert!(
        !clean.contains("TOPSECRET"),
        "must redact signed videoplayback query"
    );
    assert!(clean.contains("[REDACTED_SENSITIVE_LINE]"));
}

fn relative_luminance(rgb: [u8; 3]) -> f64 {
    let lin = |c: u8| {
        let s = c as f64 / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2])
}

fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let la = relative_luminance(a);
    let lb = relative_luminance(b);
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[test]
fn ambient_palette_extraction_caches_and_preserves_wcag_aa_contrast() {
    // Vibrant neon yellow + magenta pixels (worst-case stress test for text contrast)
    let mut pixels = Vec::with_capacity(300);
    for _ in 0..100 {
        pixels.extend_from_slice(&[255, 240, 30]);
    }
    let extracted = extract_and_cache_from_rgb("wcag-stress-track", &pixels);
    let cached = palette_for_track("wcag-stress-track", "ignored-seed");
    assert_eq!(extracted, cached, "palette must be cached by track_id");

    // Dark foreground (#f4f5f7) vs dark_backdrop must far exceed WCAG AA (4.5:1)
    let dark_cr = contrast_ratio([0xf4, 0xf5, 0xf7], extracted.dark_backdrop);
    assert!(
        dark_cr >= 10.0,
        "dark ambient backdrop contrast ratio too low: {dark_cr:.2}"
    );
    // Light foreground (#111214) vs light_backdrop must far exceed WCAG AA (4.5:1)
    let light_cr = contrast_ratio([0x11, 0x12, 0x14], extracted.light_backdrop);
    assert!(
        light_cr >= 10.0,
        "light ambient backdrop contrast ratio too low: {light_cr:.2}"
    );
}

#[test]
fn stage3_search_filters_desktop_side_panel_and_zero_blur_in_virtual_lists() -> Result<(), String> {
    resonance::store::enable_fixture_mode(true);
    let mut desktop = Headless::start(EngineConfig {
        viewport: Size::new(Points(1200.0), Points(800.0)),
        ..EngineConfig::new(Root::of::<resonance::App>())
    })
    .map_err(|e| format!("{e:?}"))?;

    // Desktop >= 1080px mounts the right-hand frosted Queue/Lyrics side panel
    desktop.wait_for("Up Next")?;
    desktop.wait_for("Synced Lyrics")?;
    desktop.press("id:side-panel-tab-lyrics")?;
    desktop.wait_for("Streetlights blur into copper and glass")?;
    desktop.press("id:side-panel-tab-queue")?;

    // Search filters (All / Songs / Albums) work deterministically
    desktop.press("id:nav-search")?;
    desktop.wait_for("All")?;
    desktop.wait_for("Songs")?;
    desktop.wait_for("Albums")?;
    desktop.press("id:search-filter-albums")?;
    desktop.press("id:search-filter-all")?;

    // Static performance guard: NEVER apply backdrop blur inside virtual_list closures
    for rel in [
        "src/screens/lyrics.rs",
        "src/screens/queue.rs",
        "src/screens/search.rs",
        "src/screens/shell.rs",
    ] {
        let src = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
            .expect("read screen source");
        if let Some(pos) = src.find("virtual_list(") {
            let after_vl = &src[pos..];
            // Check inside the virtual_list(...) call itself
            let end_hint = after_vl.find(".flex_1()").unwrap_or(after_vl.len());
            let vl_body = &after_vl[..end_hint];
            assert!(
                !vl_body.contains("backdrop_blur") && !vl_body.contains("backdrop_material"),
                "{rel} must NEVER apply backdrop blur inside virtual_list rows"
            );
        }
    }
    Ok(())
}
