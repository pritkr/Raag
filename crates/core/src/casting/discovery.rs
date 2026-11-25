//! On-demand LAN Cast Discovery (mDNS `_googlecast._tcp.local.`, `_airplay._tcp.local.`
//! and SSDP/UPnP `MediaRenderer:1`).
//!
//! Runs ONLY while the Cast Device Picker sheet is open (zero background battery/mDNS drain).
//! Enforces strict RFC1918 SSRF validation on SSDP `LOCATION` XML URLs.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use resonance_contract::{CastDevice, CastProtocol, CoreError, CoreResult, PlaybackError};
use tokio::sync::{RwLock, watch};

const MAX_UPNP_DESCRIPTION_BYTES: usize = 64 * 1024;

/// Validates an SSDP `LOCATION` URL against SSRF attacks.
///
/// Rejects loopback (`127.0.0.1`, `localhost`, `::1`), cloud metadata (`169.254.169.254`),
/// non-HTTP schemes, credentials, and public/non-RFC1918 IP addresses.
pub fn validate_ssdp_location_url(raw: &str) -> CoreResult<url::Url> {
    let parsed = url::Url::parse(raw.trim()).map_err(|e| {
        CoreError::Playback(PlaybackError::BackendUnavailable(format!(
            "invalid SSDP location URL: {e}"
        )))
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(CoreError::Playback(PlaybackError::BackendUnavailable(
            format!("disallowed SSDP scheme: {}", parsed.scheme()),
        )));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(CoreError::Playback(PlaybackError::BackendUnavailable(
            "SSDP location URL must not contain credentials".to_owned(),
        )));
    }
    let host_str = parsed.host_str().unwrap_or_default();
    if host_str.eq_ignore_ascii_case("localhost") {
        return Err(CoreError::Playback(PlaybackError::BackendUnavailable(
            "SSDP SSRF blocked: localhost".to_owned(),
        )));
    }
    let ip: IpAddr = host_str.parse().map_err(|_| {
        CoreError::Playback(PlaybackError::BackendUnavailable(format!(
            "SSDP location host must be an RFC1918 IP literal, got {host_str}"
        )))
    })?;
    let is_rfc1918 = match ip {
        IpAddr::V4(v4) => v4.is_private() && !v4.is_loopback() && !v4.is_link_local(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00,
    };
    if !is_rfc1918 {
        return Err(CoreError::Playback(PlaybackError::BackendUnavailable(
            format!("SSDP SSRF blocked non-RFC1918 IP: {ip}"),
        )));
    }
    Ok(parsed)
}

/// Session-scoped LAN discovery handle; stops all mDNS/SSDP tasks on `Drop`.
pub struct CastDiscoveryService {
    devices: Arc<RwLock<HashMap<(CastProtocol, String), CastDevice>>>,
    updates_rx: watch::Receiver<Vec<CastDevice>>,
    shutdown_tx: Option<watch::Sender<bool>>,
}

impl CastDiscoveryService {
    pub fn start() -> Self {
        let devices = Arc::new(RwLock::new(HashMap::new()));
        let (updates_tx, updates_rx) = watch::channel(Vec::new());
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

        // Task 1: mDNS (_googlecast._tcp.local. + _airplay._tcp.local.)
        {
            let devices = Arc::clone(&devices);
            let updates_tx = updates_tx.clone();
            let mut rx = shutdown_rx.clone();
            tokio::spawn(async move {
                let Ok(daemon) = ServiceDaemon::new() else {
                    return;
                };
                let cast_rx = daemon.browse("_googlecast._tcp.local.").ok();
                let airplay_rx = daemon.browse("_airplay._tcp.local.").ok();

                loop {
                    if *rx.borrow() {
                        break;
                    }
                    let mut changed = false;
                    if let Some(ref crx) = cast_rx {
                        while let Ok(ev) = crx.try_recv() {
                            if let ServiceEvent::ServiceResolved(info) = ev
                                && let Some(dev) = chromecast_from_mdns(&info)
                            {
                                devices
                                    .write()
                                    .await
                                    .insert((dev.protocol, dev.id.clone()), dev);
                                changed = true;
                            }
                        }
                    }
                    if let Some(ref arx) = airplay_rx {
                        while let Ok(ev) = arx.try_recv() {
                            if let ServiceEvent::ServiceResolved(info) = ev
                                && let Some(dev) = airplay_from_mdns(&info)
                            {
                                devices
                                    .write()
                                    .await
                                    .insert((dev.protocol, dev.id.clone()), dev);
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        emit_sorted(&devices, &updates_tx).await;
                    }
                    tokio::select! {
                        _ = rx.changed() => break,
                        _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                    }
                }
                let _ = daemon.shutdown();
            });
        }

        // Task 2: SSDP / UPnP MediaRenderer:1
        {
            let devices = Arc::clone(&devices);
            let updates_tx = updates_tx.clone();
            tokio::spawn(async move {
                let search_target = rupnp::ssdp::SearchTarget::URN(rupnp::ssdp::URN::device(
                    "schemas-upnp-org",
                    "MediaRenderer",
                    1,
                ));
                if let Ok(stream) = rupnp::discover(&search_target, Duration::from_secs(3)).await {
                    tokio::pin!(stream);
                    while let Some(res) = stream.next().await {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                        if let Ok(dev) = res {
                            let loc = dev.url().to_string();
                            if let Ok(validated) = validate_ssdp_location_url(&loc)
                                && let Some(ip) =
                                    validated.host_str().and_then(|h| h.parse::<IpAddr>().ok())
                            {
                                let _ = MAX_UPNP_DESCRIPTION_BYTES;
                                let cast_dev = CastDevice {
                                    id: format!("dlna:{loc}"),
                                    name: dev.friendly_name().to_owned(),
                                    model: None,
                                    protocol: CastProtocol::DlnaUpnp,
                                    address: ip,
                                    port: validated.port_or_known_default().unwrap_or(80),
                                    control_url: Some(loc),
                                };
                                devices
                                    .write()
                                    .await
                                    .insert((cast_dev.protocol, cast_dev.id.clone()), cast_dev);
                                emit_sorted(&devices, &updates_tx).await;
                            }
                        }
                    }
                }
                let _ = shutdown_rx.changed().await;
            });
        }

        Self {
            devices,
            updates_rx,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<Vec<CastDevice>> {
        self.updates_rx.clone()
    }

    pub async fn snapshot(&self) -> Vec<CastDevice> {
        let map = self.devices.read().await;
        let mut list: Vec<CastDevice> = map.values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        list
    }

    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
    }
}

impl Drop for CastDiscoveryService {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn emit_sorted(
    devices: &RwLock<HashMap<(CastProtocol, String), CastDevice>>,
    tx: &watch::Sender<Vec<CastDevice>>,
) {
    let map = devices.read().await;
    let mut list: Vec<CastDevice> = map.values().cloned().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    let _ = tx.send(list);
}

fn chromecast_from_mdns(info: &mdns_sd::ServiceInfo) -> Option<CastDevice> {
    let addr = info
        .get_addresses()
        .iter()
        .find(|a| a.is_ipv4())
        .copied()
        .or_else(|| info.get_addresses().iter().next().copied())?;
    let props = info.get_properties();
    // Filter out non-audio groups if capability bitmask `ca` lacks audio-out (bit 2 = 0x04)
    if let Some(ca_val) = props.get_property_val_str("ca")
        && let Ok(ca) = ca_val.parse::<u32>()
        && (ca & 0x04) == 0
    {
        return None;
    }
    let id = props
        .get_property_val_str("id")
        .unwrap_or_else(|| info.get_fullname())
        .to_owned();
    let name = props
        .get_property_val_str("fn")
        .unwrap_or_else(|| info.get_fullname())
        .to_owned();
    let model = props.get_property_val_str("md").map(str::to_owned);
    Some(CastDevice {
        id,
        name,
        model,
        protocol: CastProtocol::Chromecast,
        address: addr,
        port: if info.get_port() == 0 {
            8009
        } else {
            info.get_port()
        },
        control_url: None,
    })
}

fn airplay_from_mdns(info: &mdns_sd::ServiceInfo) -> Option<CastDevice> {
    let addr = info.get_addresses().iter().next().copied()?;
    Some(CastDevice {
        id: format!("airplay:{}", info.get_fullname()),
        name: info
            .get_fullname()
            .trim_end_matches("._airplay._tcp.local.")
            .to_owned(),
        model: info
            .get_properties()
            .get_property_val_str("model")
            .map(str::to_owned),
        protocol: CastProtocol::AirPlayNative,
        address: addr,
        port: info.get_port(),
        control_url: None,
    })
}
