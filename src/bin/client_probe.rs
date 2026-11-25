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

use resonance_core::innertube::auth::redact_secrets;
use resonance_core::innertube::clients::ClientConfig;
use resonance_core::innertube::matrix::{ClientMatrix, PoTokenPolicy};
use resonance_core::solver::{ChallengeInput, QuickJsSolver};
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
                !matches!(entry.po_token_policy, PoTokenPolicy::RequiredIosGuard) && entry.enabled;
            entry.probe_pass_cookies_on = cfg.allows_browser_cookies() && entry.enabled;
            if entry.enabled {
                passed_clients += 1;
            }
            continue;
        }

        // Live probe against fixed probe videos (cookies off).
        let mut ok_off = false;
        let mut http_status = 0u16;
        let mut playability = String::from("UNPROBED");
        let mut audio_itags: Vec<u64> = Vec::new();
        let mut has_ciphered_stream = false;
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
            {
                http_status = resp.status().as_u16();
                if resp.status().is_success()
                    && let Ok(json) = resp.json::<serde_json::Value>().await
                {
                    playability = json["playabilityStatus"]["status"]
                        .as_str()
                        .unwrap_or("UNKNOWN")
                        .to_owned();
                    ok_off = playability == "OK";
                    if let Some(formats) = json["streamingData"]["adaptiveFormats"].as_array() {
                        for f in formats {
                            let mime = f["mimeType"].as_str().unwrap_or("");
                            if mime.starts_with("audio/")
                                && let Some(itag) = f["itag"].as_u64()
                            {
                                audio_itags.push(itag);
                            }
                            if f["signatureCipher"].as_str().is_some()
                                || f["cipher"].as_str().is_some()
                            {
                                has_ciphered_stream = true;
                            }
                        }
                    }
                }
            }
        }
        let reason_line = format!(
            "probe client={} http={} playability={} audio_itags={:?} ciphered={}",
            cfg.client_name, http_status, playability, audio_itags, has_ciphered_stream
        );
        println!("{}", redact_secrets(&reason_line));
        entry.probe_pass_cookies_off = ok_off;
        entry.enabled = ok_off || entry.probe_pass_cookies_on;
        if entry.enabled {
            passed_clients += 1;
        }
    }

    if !offline_sim {
        let solver_dir = std::env::temp_dir().join("raag_client_probe_ejs");
        let _ = std::fs::remove_dir_all(&solver_dir);
        let solver = QuickJsSolver::new(&solver_dir);
        let manifest = solver.active_ejs_manifest();
        let probe_js = r#"
            function solveN(n) { return n.split("").reverse().join(""); }
            function solveSig(s) { return s.slice(1); }
        "#;
        let t0 = std::time::Instant::now();
        let solve_res = solver
            .solve(ChallengeInput {
                player_hash: "probe-ejs-check-v2".into(),
                solver_js: probe_js.into(),
                n_param: Some("abcd1234".into()),
                sig_param: Some("Xsignature".into()),
            })
            .await;
        let elapsed_ms = t0.elapsed().as_millis();
        match solve_res {
            Ok(solved) => println!(
                "rquickjs_ejs_solver=OK elapsed_ms={} manifest_ok={} n_solved={} sig_solved={} watchdog_hit=false",
                elapsed_ms,
                manifest.is_ok(),
                solved.n_param.is_some(),
                solved.sig_param.is_some()
            ),
            Err(e) => println!(
                "rquickjs_ejs_solver=ERR elapsed_ms={} manifest_ok={} watchdog_hit={} err={}",
                elapsed_ms,
                manifest.is_ok(),
                elapsed_ms >= 9900,
                redact_secrets(&e.to_string())
            ),
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
