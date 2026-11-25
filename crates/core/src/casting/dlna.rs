//! DLNA / UPnP MediaRenderer (`AVTransport:1` & `RenderingControl:1`) transport.
//!
//! Includes strict XML escaping for DIDL-Lite metadata and SSRF validation via
//! [`crate::casting::discovery::validate_ssdp_location_url`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use resonance_contract::{
    CastDevice, CastMediaPayload, CastTransport, CoreError, CoreResult, NowPlayingMetadata,
    PlaybackError,
};
use tokio::sync::Mutex;

use crate::casting::discovery::validate_ssdp_location_url;

const AV_TRANSPORT_SERVICE: &str = "urn:schemas-upnp-org:service:AVTransport:1";
const RENDERING_CONTROL_SERVICE: &str = "urn:schemas-upnp-org:service:RenderingControl:1";
const MAX_CONSECUTIVE_TIMEOUTS: u32 = 3;

/// Escapes XML special characters (`& < > " '`) for DIDL-Lite & SOAP payloads.
pub fn xml_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// Builds a properly XML-escaped DIDL-Lite `<item>` for `AVTransport::SetAVTransportURI`.
pub fn build_didl_lite_metadata(
    stream_url: &str,
    meta: &NowPlayingMetadata,
    content_type: &str,
) -> String {
    let title = xml_escape(&meta.title);
    let artist = xml_escape(&meta.artist);
    let album = xml_escape(meta.album.as_deref().unwrap_or(""));
    let url_esc = xml_escape(stream_url);
    let mime_esc = xml_escape(content_type);
    let art_tag = meta
        .artwork_url
        .as_deref()
        .map(|u| format!("<upnp:albumArtURI>{}</upnp:albumArtURI>", xml_escape(u)))
        .unwrap_or_default();

    format!(
        r#"<DIDL-Lite xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:upnp="urn:schemas-upnp-org:metadata-1-0/upnp/"><item id="0" parentID="-1" restricted="1"><dc:title>{title}</dc:title><upnp:artist>{artist}</upnp:artist><upnp:album>{album}</upnp:album>{art_tag}<upnp:class>object.item.audioItem.musicTrack</upnp:class><res protocolInfo="http-get:*:{mime_esc}:*">{url_esc}</res></item></DIDL-Lite>"#
    )
}

fn format_rel_time(ms: u64) -> String {
    let total_secs = ms / 1000;
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

fn parse_rel_time_ms(raw: &str) -> Option<u64> {
    let parts: Vec<&str> = raw.trim().split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: u64 = parts[0].parse().ok()?;
    let m: u64 = parts[1].parse().ok()?;
    let s: u64 = parts[2].split('.').next()?.parse().ok()?;
    Some((h * 3600 + m * 60 + s) * 1000)
}

pub struct DlnaTransport {
    http: reqwest::Client,
    control_url: Mutex<Option<String>>,
    consecutive_timeouts: Arc<AtomicU32>,
}

impl Default for DlnaTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl DlnaTransport {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            control_url: Mutex::new(None),
            consecutive_timeouts: Arc::new(AtomicU32::new(0)),
        }
    }

    async fn soap_call(
        &self,
        service_urn: &str,
        action: &str,
        inner_xml: &str,
    ) -> CoreResult<String> {
        let url = self.control_url.lock().await.clone().ok_or_else(|| {
            CoreError::Playback(PlaybackError::BackendUnavailable(
                "DLNA not connected".into(),
            ))
        })?;

        let envelope = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><s:Body><u:{action} xmlns:u="{service_urn}">{inner_xml}</u:{action}></s:Body></s:Envelope>"#
        );

        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "text/xml; charset=\"utf-8\"")
            .header("SOAPACTION", format!("\"{service_urn}#{action}\""))
            .body(envelope)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                self.consecutive_timeouts.store(0, Ordering::Relaxed);
                r.text().await.map_err(|e| {
                    CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string()))
                })
            }
            Ok(r) => Err(CoreError::Playback(PlaybackError::BackendUnavailable(
                format!("DLNA SOAP {action} returned HTTP {}", r.status()),
            ))),
            Err(e) => {
                let count = self.consecutive_timeouts.fetch_add(1, Ordering::Relaxed) + 1;
                if count >= MAX_CONSECUTIVE_TIMEOUTS {
                    *self.control_url.lock().await = None;
                }
                Err(CoreError::Playback(PlaybackError::BackendUnavailable(
                    e.to_string(),
                )))
            }
        }
    }
}

#[async_trait]
impl CastTransport for DlnaTransport {
    async fn connect(&self, device: &CastDevice) -> CoreResult<()> {
        let raw_url = device.control_url.as_deref().ok_or_else(|| {
            CoreError::Playback(PlaybackError::BackendUnavailable(
                "missing UPnP control_url".to_owned(),
            ))
        })?;
        let validated = validate_ssdp_location_url(raw_url)?;
        self.consecutive_timeouts.store(0, Ordering::Relaxed);
        *self.control_url.lock().await = Some(validated.to_string());
        Ok(())
    }

    async fn load_media(&self, payload: CastMediaPayload) -> CoreResult<()> {
        let didl = build_didl_lite_metadata(
            &payload.stream_url,
            &payload.metadata,
            &payload.content_type,
        );
        let uri_esc = xml_escape(&payload.stream_url);
        let didl_esc = xml_escape(&didl);
        self.soap_call(
            AV_TRANSPORT_SERVICE,
            "SetAVTransportURI",
            &format!(
                "<InstanceID>0</InstanceID><CurrentURI>{uri_esc}</CurrentURI><CurrentURIMetaData>{didl_esc}</CurrentURIMetaData>"
            ),
        )
        .await?;
        if payload.autoplay {
            self.play().await?;
        }
        if payload.start_position_ms > 0 {
            let _ = self.seek_ms(payload.start_position_ms).await;
        }
        Ok(())
    }

    async fn play(&self) -> CoreResult<()> {
        self.soap_call(
            AV_TRANSPORT_SERVICE,
            "Play",
            "<InstanceID>0</InstanceID><Speed>1</Speed>",
        )
        .await
        .map(|_| ())
    }

    async fn pause(&self) -> CoreResult<()> {
        self.soap_call(AV_TRANSPORT_SERVICE, "Pause", "<InstanceID>0</InstanceID>")
            .await
            .map(|_| ())
    }

    async fn seek_ms(&self, position_ms: u64) -> CoreResult<()> {
        let target = format_rel_time(position_ms);
        self.soap_call(
            AV_TRANSPORT_SERVICE,
            "Seek",
            &format!("<InstanceID>0</InstanceID><Unit>REL_TIME</Unit><Target>{target}</Target>"),
        )
        .await
        .map(|_| ())
    }

    async fn set_volume(&self, volume: f32) -> CoreResult<()> {
        let pct = (volume.clamp(0.0, 1.0) * 100.0).round() as u32;
        self.soap_call(
            RENDERING_CONTROL_SERVICE,
            "SetVolume",
            &format!("<InstanceID>0</InstanceID><Channel>Master</Channel><DesiredVolume>{pct}</DesiredVolume>"),
        )
        .await
        .map(|_| ())
    }

    async fn set_muted(&self, muted: bool) -> CoreResult<()> {
        let val = if muted { 1 } else { 0 };
        self.soap_call(
            RENDERING_CONTROL_SERVICE,
            "SetMute",
            &format!("<InstanceID>0</InstanceID><Channel>Master</Channel><DesiredMute>{val}</DesiredMute>"),
        )
        .await
        .map(|_| ())
    }

    async fn poll_status(&self) -> CoreResult<Option<(bool, u64)>> {
        let transport_xml = self
            .soap_call(
                AV_TRANSPORT_SERVICE,
                "GetTransportInfo",
                "<InstanceID>0</InstanceID>",
            )
            .await?;
        let is_playing =
            transport_xml.contains("<CurrentTransportState>PLAYING</CurrentTransportState>");
        let pos_xml = self
            .soap_call(
                AV_TRANSPORT_SERVICE,
                "GetPositionInfo",
                "<InstanceID>0</InstanceID>",
            )
            .await?;
        let pos_ms = extract_xml_tag(&pos_xml, "RelTime")
            .and_then(parse_rel_time_ms)
            .unwrap_or(0);
        Ok(Some((is_playing, pos_ms)))
    }

    async fn disconnect(&self) -> CoreResult<()> {
        let _ = self
            .soap_call(AV_TRANSPORT_SERVICE, "Stop", "<InstanceID>0</InstanceID>")
            .await;
        *self.control_url.lock().await = None;
        Ok(())
    }
}

fn extract_xml_tag<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(&xml[start..end])
}
