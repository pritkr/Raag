//! SponsorBlock (`music_offtopic`) integration using privacy-preserving SHA-256 prefix lookup.

use std::time::Duration;

use async_trait::async_trait;
use resonance_contract::{CoreResult, SponsorBlockProvider, SponsorSegment};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const SPONSORBLOCK_API: &str = "https://sponsor.ajay.app/api/skipSegments";
const CATEGORIES: &str = r#"["music_offtopic","sponsor","selfpromo"]"#;

pub struct HttpSponsorBlockProvider {
    http: reqwest::Client,
}

impl HttpSponsorBlockProvider {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        Self { http }
    }

    /// Computes the 4-hex-char SHA-256 prefix sent to SponsorBlock (privacy-preserving).
    pub fn hash_prefix(video_id: &str) -> String {
        let digest = Sha256::digest(video_id.as_bytes());
        format!("{:02x}{:02x}", digest[0], digest[1])
    }
}

impl Default for HttpSponsorBlockProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Deserialize)]
struct ApiVideo {
    #[serde(rename = "videoID")]
    video_id: String,
    segments: Vec<ApiSegment>,
}

#[derive(Deserialize)]
struct ApiSegment {
    segment: [f64; 2],
    #[serde(rename = "actionType")]
    action_type: String,
    #[serde(rename = "videoDuration", default)]
    video_duration: f64,
}

pub fn parse_sponsorblock_json(video_id: &str, body: &str) -> Vec<SponsorSegment> {
    let Ok(videos) = serde_json::from_str::<Vec<ApiVideo>>(body) else {
        return Vec::new();
    };
    let mut out: Vec<SponsorSegment> = videos
        .into_iter()
        .filter(|v| v.video_id == video_id)
        .flat_map(|v| v.segments)
        .filter(|s| s.action_type == "skip" && s.segment[1] > s.segment[0] + 0.5)
        .map(|s| SponsorSegment {
            start_secs: s.segment[0],
            end_secs: s.segment[1],
            video_duration_secs: s.video_duration,
        })
        .collect();
    out.sort_by(|a, b| a.start_secs.total_cmp(&b.start_secs));
    out
}

#[async_trait]
impl SponsorBlockProvider for HttpSponsorBlockProvider {
    async fn fetch_segments(&self, video_id: &str) -> CoreResult<Vec<SponsorSegment>> {
        let prefix = Self::hash_prefix(video_id);
        let url = format!("{SPONSORBLOCK_API}/{prefix}");
        let Ok(resp) = self
            .http
            .get(&url)
            .query(&[("categories", CATEGORIES), ("actionType", "skip")])
            .send()
            .await
        else {
            return Ok(Vec::new());
        };
        if resp.status() == reqwest::StatusCode::NOT_FOUND || !resp.status().is_success() {
            return Ok(Vec::new());
        }
        let body = resp.text().await.unwrap_or_default();
        Ok(parse_sponsorblock_json(video_id, &body))
    }
}
