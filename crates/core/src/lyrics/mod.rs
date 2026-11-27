//! Synced Karaoke Lyrics Cascade:
//! Priority order:
//! 1. SQLite 30-day cache (instant)
//! 2. LRCLIB synced/plain (`https://lrclib.net/api/get`)
//! 3. Better Lyrics Unison (`https://unison.betterlyrics.org/lyrics`)
//! 4. YouTube Music internal lyrics (`MusicProvider::ytm_lyrics`)
//!
//! Includes a pure LRC timestamp parser supporting `[mm:ss.xx]` and `[mm:ss.xxx]` lines.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use resonance_contract::{
    CacheStore, CoreResult, LyricLine, LyricSyncKind, LyricsDocument, LyricsProvider,
    MusicProvider, Track,
};
use serde_json::Value;

use crate::unix_secs;

const TTL_LYRICS_SECS: u64 = 30 * 24 * 3600; // 30 days
const LRCLIB_GET: &str = "https://lrclib.net/api/get";
const BETTER_LYRICS_UNISON: &str = "https://unison.betterlyrics.org/lyrics";

pub struct CascadeLyricsProvider {
    http: reqwest::Client,
    music: Arc<dyn MusicProvider>,
    cache: Arc<dyn CacheStore>,
}

impl CascadeLyricsProvider {
    pub fn new(music: Arc<dyn MusicProvider>, cache: Arc<dyn CacheStore>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(6))
            .user_agent("Raag/0.1 (Pure Rust NSH YouTube Music Client)")
            .build()
            .unwrap_or_default();
        Self { http, music, cache }
    }

    async fn fetch_lrclib(&self, track: &Track) -> Option<LyricsDocument> {
        let artist = track.artist_display();
        let duration_secs = (track.duration_ms / 1000).to_string();
        let mut params = vec![
            ("track_name", track.title.as_str()),
            ("artist_name", artist.as_str()),
        ];
        if track.duration_ms > 0 {
            params.push(("duration", duration_secs.as_str()));
        }
        let resp = self.http.get(LRCLIB_GET).query(&params).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let json: Value = resp.json().await.ok()?;
        if let Some(synced) = json.get("syncedLyrics").and_then(Value::as_str) {
            let lines = parse_lrc(synced);
            if !lines.is_empty() {
                return Some(LyricsDocument {
                    video_id: track.id.clone(),
                    provider: "LRCLIB".to_owned(),
                    sync_kind: LyricSyncKind::Line,
                    lines,
                });
            }
        }
        if let Some(plain) = json.get("plainLyrics").and_then(Value::as_str) {
            let lines = parse_plain_lyrics(plain);
            if !lines.is_empty() {
                return Some(LyricsDocument {
                    video_id: track.id.clone(),
                    provider: "LRCLIB (Plain)".to_owned(),
                    sync_kind: LyricSyncKind::Plain,
                    lines,
                });
            }
        }
        None
    }

    async fn fetch_better_lyrics(&self, track: &Track) -> Option<LyricsDocument> {
        let artist = track.artist_display();
        let resp = self
            .http
            .get(BETTER_LYRICS_UNISON)
            .query(&[
                ("s", track.title.as_str()),
                ("a", artist.as_str()),
                ("d", &(track.duration_ms / 1000).to_string()),
            ])
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let json: Value = resp.json().await.ok()?;
        if let Some(lrc) = json.get("lrc").and_then(Value::as_str) {
            let lines = parse_lrc(lrc);
            if !lines.is_empty() {
                return Some(LyricsDocument {
                    video_id: track.id.clone(),
                    provider: "Better Lyrics".to_owned(),
                    sync_kind: LyricSyncKind::Line,
                    lines,
                });
            }
        }
        None
    }
}

#[async_trait]
impl LyricsProvider for CascadeLyricsProvider {
    async fn fetch_lyrics(&self, track: &Track) -> CoreResult<Option<LyricsDocument>> {
        let cache_key = format!("lyrics:{}", track.id);
        let now = unix_secs();

        if let Ok(Some((raw, false))) = self.cache.get_metadata(&cache_key, now) {
            if let Ok(doc) = serde_json::from_str::<LyricsDocument>(&raw) {
                return Ok(Some(doc));
            }
            let _ = self.cache.evict_metadata(&cache_key);
        }

        // Cascade: try YTM internal lyrics, then LRCLIB (prefer synced), then Better Lyrics.
        let ytm_doc = self.music.ytm_lyrics(&track.id).await.ok().flatten();
        let synced_doc = if let Some(lrc) = self.fetch_lrclib(track).await {
            if lrc.sync_kind != LyricSyncKind::Plain {
                Some(lrc)
            } else {
                self.fetch_better_lyrics(track).await.or(Some(lrc))
            }
        } else {
            self.fetch_better_lyrics(track).await
        };

        let chosen = synced_doc.or(ytm_doc);
        if let Some(doc) = &chosen
            && let Ok(serialized) = serde_json::to_string(doc)
        {
            let _ = self
                .cache
                .put_metadata(&cache_key, &serialized, TTL_LYRICS_SECS, now);
        }
        Ok(chosen)
    }
}

/// Parses standard LRC synced lyrics (`[mm:ss.xx] Line text`) into ordered `LyricLine`s.
pub fn parse_lrc(input: &str) -> Vec<LyricLine> {
    let mut parsed: Vec<(u64, String)> = Vec::new();
    for raw_line in input.lines() {
        let mut rest = raw_line.trim();
        let mut timestamps = Vec::new();
        while let Some(after_open) = rest.strip_prefix('[') {
            let Some((tag, remainder)) = after_open.split_once(']') else {
                break;
            };
            if let Some(ms) = parse_lrc_timestamp(tag) {
                timestamps.push(ms);
            }
            rest = remainder.trim_start();
        }
        let text = rest.trim().to_owned();
        for ts in timestamps {
            parsed.push((ts, text.clone()));
        }
    }
    parsed.sort_by_key(|(ts, _)| *ts);

    let mut lines = Vec::with_capacity(parsed.len());
    for i in 0..parsed.len() {
        let (start_ms, text) = &parsed[i];
        let end_ms = parsed
            .get(i + 1)
            .map(|(next_ts, _)| *next_ts)
            .unwrap_or(start_ms + 4000);
        let is_instrumental = text.is_empty() || text == "♪" || text == "···";
        lines.push(LyricLine {
            start_ms: *start_ms,
            end_ms,
            text: if is_instrumental {
                "♪  •  ♪".to_owned()
            } else {
                text.clone()
            },
            words: Vec::new(),
            is_instrumental,
        });
    }
    lines
}

fn parse_lrc_timestamp(tag: &str) -> Option<u64> {
    let (mins_s, secs_s) = tag.split_once(':')?;
    let mins: u64 = mins_s.trim().parse().ok()?;
    let (whole_s, frac_s) = secs_s
        .split_once('.')
        .or_else(|| secs_s.split_once(':'))
        .unwrap_or((secs_s, "0"));
    let secs: u64 = whole_s.trim().parse().ok()?;
    let frac_clean = frac_s.trim();
    let ms: u64 = match frac_clean.len() {
        0 => 0,
        1 => frac_clean.parse::<u64>().ok()? * 100,
        2 => frac_clean.parse::<u64>().ok()? * 10,
        _ => frac_clean[..3].parse::<u64>().ok()?,
    };
    Some((mins * 60 + secs) * 1000 + ms)
}

pub fn parse_plain_lyrics(input: &str) -> Vec<LyricLine> {
    input
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| LyricLine {
            start_ms: 0,
            end_ms: 0,
            text: l.to_owned(),
            words: Vec::new(),
            is_instrumental: false,
        })
        .collect()
}
