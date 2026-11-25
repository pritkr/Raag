//! InnerTube `/youtubei/v1/player` endpoint parser & format selection.

use resonance_contract::{AudioCodec, CoreError, CoreResult, ProviderError, StreamInfo};
use serde_json::Value;

/// Candidate format extracted from `/player` `streamingData.adaptiveFormats`.
#[derive(Debug, Clone)]
pub struct RawAudioFormat {
    pub itag: u32,
    pub url: Option<String>,
    pub signature_cipher: Option<String>,
    pub mime_type: String,
    pub codec: AudioCodec,
    pub bitrate_kbps: u32,
    pub content_length: u64,
    pub duration_ms: u64,
    pub loudness_db: Option<f32>,
}

/// Preferred audio itag order per RESEARCH.md Rule #4:
/// Opus 774 (~256 kbps Premium) -> Opus 251 (~128-160 kbps Free) -> AAC 141 -> AAC 140 -> HE-AAC 139.
pub const PREFERRED_AUDIO_ITAGS: &[u32] = &[774, 251, 141, 140, 139];

/// Parses `/player` response and picks the highest-priority audio format for Android/Premium default.
pub fn parse_player_response(
    video_id: &str,
    client_name: &str,
    resp: &Value,
    now_unix_secs: u64,
) -> CoreResult<RawAudioFormat> {
    parse_player_response_for_platform(
        video_id,
        client_name,
        resp,
        now_unix_secs,
        crate::innertube::formats::TargetPlatform::Android,
        crate::innertube::matrix::AccountTier::PremiumSignedIn,
    )
}

/// Parses `/player` response gated by native target decoder support (`TargetPlatform`)
/// and account tier (`AccountTier::Anonymous | FreeSignedIn | PremiumSignedIn`).
pub fn parse_player_response_for_platform(
    video_id: &str,
    client_name: &str,
    resp: &Value,
    now_unix_secs: u64,
    platform: crate::innertube::formats::TargetPlatform,
    tier: crate::innertube::matrix::AccountTier,
) -> CoreResult<RawAudioFormat> {
    let playability = &resp["playabilityStatus"];
    let status = playability["status"].as_str().unwrap_or("ERROR");
    if status != "OK" {
        return Err(CoreError::Provider(ProviderError::Unplayable {
            status: status.to_owned(),
            reason: playability["reason"]
                .as_str()
                .unwrap_or("video unavailable")
                .to_owned(),
        }));
    }

    let formats = resp
        .pointer("/streamingData/adaptiveFormats")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CoreError::Provider(ProviderError::NoStream {
                video_id: video_id.to_owned(),
            })
        })?;

    let track_loudness = resp
        .pointer("/playerConfig/audioConfig/loudnessDb")
        .and_then(Value::as_f64)
        .map(|v| v as f32);

    let _ = (client_name, now_unix_secs);
    let preferred_itags = platform.preferred_audio_itags(tier);

    for &itag in preferred_itags {
        if let Some(f) = formats
            .iter()
            .find(|f| f.get("itag").and_then(Value::as_u64) == Some(u64::from(itag)))
        {
            let mime_type = f
                .get("mimeType")
                .and_then(Value::as_str)
                .unwrap_or("audio/webm")
                .to_owned();
            let codec = if matches!(itag, 774 | 251) || mime_type.contains("opus") {
                AudioCodec::Opus
            } else {
                AudioCodec::Aac
            };
            let bitrate_kbps = f
                .get("averageBitrate")
                .or_else(|| f.get("bitrate"))
                .and_then(Value::as_u64)
                .unwrap_or(128_000) as u32
                / 1000;
            let content_length = f
                .get("contentLength")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let duration_ms = f
                .get("approxDurationMs")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let loudness_db = f
                .get("loudnessDb")
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .or(track_loudness);

            return Ok(RawAudioFormat {
                itag,
                url: f.get("url").and_then(Value::as_str).map(str::to_owned),
                signature_cipher: f
                    .get("signatureCipher")
                    .or_else(|| f.get("cipher"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                mime_type,
                codec,
                bitrate_kbps,
                content_length,
                duration_ms,
                loudness_db,
            });
        }
    }

    Err(CoreError::Provider(ProviderError::NoStream {
        video_id: video_id.to_owned(),
    }))
}

pub fn finalize_stream_info(
    video_id: &str,
    client_name: &str,
    raw: RawAudioFormat,
    resolved_url: String,
    now_unix_secs: u64,
) -> StreamInfo {
    // Cap cached stream TTL to <= 4h per RESEARCH.md Section 4.
    let expires_at_unix_secs = now_unix_secs + 4 * 3600;
    StreamInfo {
        video_id: video_id.to_owned(),
        url: resolved_url,
        itag: raw.itag,
        mime_type: raw.mime_type,
        codec: raw.codec,
        bitrate_kbps: raw.bitrate_kbps,
        content_length: raw.content_length,
        duration_ms: raw.duration_ms,
        loudness_db: raw.loudness_db,
        resolved_by_client: client_name.to_owned(),
        expires_at_unix_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_loudness_db_and_selects_preferred_format() {
        let resp = json!({
            "playabilityStatus": {"status": "OK"},
            "playerConfig": {"audioConfig": {"loudnessDb": -2.4}},
            "streamingData": {
                "adaptiveFormats": [
                    {
                        "itag": 140,
                        "url": "https://cdn.example/140",
                        "mimeType": "audio/mp4; codecs=\"mp4a.40.2\"",
                        "bitrate": 128000,
                        "contentLength": "4096000",
                        "approxDurationMs": "200000"
                    },
                    {
                        "itag": 251,
                        "url": "https://cdn.example/251",
                        "mimeType": "audio/webm; codecs=\"opus\"",
                        "bitrate": 160000,
                        "contentLength": "5120000",
                        "approxDurationMs": "200000"
                    }
                ]
            }
        });
        let raw = parse_player_response("vid_1", "ANDROID_VR", &resp, 1_000).unwrap();
        assert_eq!(raw.itag, 251);
        assert_eq!(raw.codec, AudioCodec::Opus);
        assert_eq!(raw.loudness_db, Some(-2.4));
        let info = finalize_stream_info(
            "vid_1",
            "ANDROID_VR",
            raw,
            "https://cdn.example/251".into(),
            1_000,
        );
        assert_eq!(info.expires_at_unix_secs, 1_000 + 4 * 3600);
    }
}
