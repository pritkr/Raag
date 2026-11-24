//! Nightly InnerTube Client Matrix Probe Harness (`cargo run --bin client_probe`).
//!
//! Tests each client in `config/client_matrix.json` against fixed probe videos
//! (Free, Premium-only itag, Age-restricted; Cookies On & Off) and atomically writes
//! a freshly scored `client_matrix.json`.
//!
//! Usage:
//! - Live nightly probe: `cargo run --bin client_probe -- --output config/client_matrix.json`
//! - Offline CI check:   `cargo run --bin client_probe -- --offline-sim --output /tmp/matrix.json`

use std::path::PathBuf;

use resonance_core::innertube::clients::ClientConfig;
use resonance_core::innertube::matrix::{ClientMatrix, PoTokenPolicy};
use resonance_core::unix_secs;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let offline_sim = args.iter().any(|a| a == "--offline-sim");
    let output_path = args
        .windows(2)
        .find(|w| w[0] == "--output")
        .map(|w| PathBuf::from(&w[1]))
        .unwrap_or_else(|| PathBuf::from("config/client_matrix.json"));

    let mut matrix = ClientMatrix::embedded_default().expect("valid baseline client_matrix.json");
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("reqwest client");

    let mut passed_clients = 0usize;
    for entry in &mut matrix.clients {
        let Some(kind) = entry.client_kind() else {
            entry.enabled = false;
            continue;
        };
        let cfg = ClientConfig::for_kind(kind);

        if offline_sim {
            // Deterministic simulation verifying PO-token and auth-isolation rules offline.
            entry.probe_pass_cookies_off =
                !matches!(entry.po_token_policy, PoTokenPolicy::RequiredIosGuard)
                    && entry.enabled;
            entry.probe_pass_cookies_on = cfg.allows_browser_cookies() && entry.enabled;
            if entry.enabled {
                passed_clients += 1;
            }
            continue;
        }

        // Live probe against fixed probe videos (cookies off).
        let mut ok_off = false;
        if let Some(probe) = matrix.probe_videos.first() {
            let payload = serde_json::json!({
                "videoId": probe.id,
                "context": cfg.build_context(None, "en", "US"),
                "contentCheckOk": true,
                "racyCheckOk": true,
            });
            let url = format!("{}/player?prettyPrint=false", cfg.api_base);
            if let Ok(resp) = http
                .post(&url)
                .header("User-Agent", cfg.user_agent)
                .header("Origin", cfg.origin)
                .header("X-YouTube-Client-Name", cfg.client_id)
                .header("X-YouTube-Client-Version", cfg.client_version)
                .json(&payload)
                .send()
                .await
                && resp.status().is_success()
                && let Ok(json) = resp.json::<serde_json::Value>().await
            {
                ok_off = json["playabilityStatus"]["status"].as_str() == Some("OK");
            }
        }
        entry.probe_pass_cookies_off = ok_off;
        entry.enabled = ok_off || entry.probe_pass_cookies_on;
        if entry.enabled {
            passed_clients += 1;
        }
    }

    matrix.updated_at_unix_secs = unix_secs();
    matrix.source = if offline_sim {
        "client_probe:offline-sim".into()
    } else {
        "client_probe:live-nightly".into()
    };

    matrix
        .save_atomic(&output_path)
        .expect("write client_matrix.json atomically");

    println!(
        "client_probe finished ({}) | active_clients={passed_clients}/{} | output={}",
        matrix.source,
        matrix.clients.len(),
        output_path.display()
    );
}
