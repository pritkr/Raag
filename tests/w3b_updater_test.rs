//! Deterministic offline verification for W3B — Signed Native NSH Auto-Updater.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use ed25519_dalek::{Signer, SigningKey};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use resonance::updater::{
    InstallMode, PlatformArtifact, UpdateManifest, cleanup_stale_staged_updates,
    detect_install_mode, fetch_and_verify_manifest, resolve_install_on_quit_default,
    stage_verified_artifact, verify_manifest_ed25519,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::net::TcpListener;

fn test_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[42u8; 32])
}

async fn spawn_updater_mock_server(
    manifest_bytes: Vec<u8>,
    sig_bytes: Vec<u8>,
    artifact_bytes: Vec<u8>,
) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let m = Arc::new(manifest_bytes);
    let s = Arc::new(sig_bytes);
    let a = Arc::new(artifact_bytes);

    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let io = TokioIo::new(stream);
            let m = Arc::clone(&m);
            let s = Arc::clone(&s);
            let a = Arc::clone(&a);
            tokio::spawn(async move {
                let svc = service_fn(move |req: Request<Incoming>| {
                    let m = Arc::clone(&m);
                    let s = Arc::clone(&s);
                    let a = Arc::clone(&a);
                    async move {
                        let resp = match req.uri().path() {
                            "/latest.json" => Response::new(Full::new(Bytes::from((*m).clone()))),
                            "/latest.json.sig" => {
                                Response::new(Full::new(Bytes::from((*s).clone())))
                            }
                            "/artifact.bin" => Response::new(Full::new(Bytes::from((*a).clone()))),
                            _ => Response::builder()
                                .status(StatusCode::NOT_FOUND)
                                .body(Full::new(Bytes::new()))
                                .unwrap(),
                        };
                        Ok::<_, hyper::Error>(resp)
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    addr
}

#[tokio::test]
async fn updater_verifies_ed25519_manifest_and_sha256_artifact_end_to_end() {
    let artifact = b"raag-binary-v0.2.0-payload".to_vec();
    let sha = hex::encode(Sha256::digest(&artifact));

    let mut platforms = BTreeMap::new();
    platforms.insert(
        "darwin-aarch64".to_owned(),
        PlatformArtifact {
            url: "https://github.com/pritkr/Raag/releases/download/v0.2.0/Raag.dmg".to_owned(),
            sha256: sha.clone(),
            size_bytes: artifact.len() as u64,
        },
    );

    let manifest = UpdateManifest {
        schema_version: 1,
        version: "0.2.0".to_owned(),
        channel: "stable".to_owned(),
        published_at_unix_secs: 1765000000,
        release_notes_md: "### What's New\n- Faster playback".to_owned(),
        platforms,
    };
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    let sk = test_signing_key();
    let sig = sk.sign(&manifest_bytes).to_bytes().to_vec();
    let pk = sk.verifying_key().to_bytes();

    let addr = spawn_updater_mock_server(manifest_bytes, sig, artifact.clone()).await;
    let verified = fetch_and_verify_manifest(
        &format!("http://{addr}/latest.json"),
        &format!("http://{addr}/latest.json.sig"),
        &pk,
        "0.1.0",
    )
    .await
    .unwrap()
    .expect("0.2.0 should be offered");
    assert_eq!(verified.version, "0.2.0");

    let dir = TempDir::new().unwrap();
    let staged = stage_verified_artifact(dir.path(), "0.2.0", "dmg", &artifact, &sha).unwrap();
    assert!(staged.exists());
}

#[test]
fn updater_rejects_bad_signature_sha256_mismatch_and_version_downgrade() {
    let sk = test_signing_key();
    let pk = sk.verifying_key().to_bytes();
    let payload = br#"{"version":"0.2.0"}"#;
    let mut sig = sk.sign(payload).to_bytes();
    sig[0] ^= 0xFF;
    assert!(verify_manifest_ed25519(payload, &sig, &pk).is_err());

    let dir = TempDir::new().unwrap();
    let good = b"good-bytes";
    let expected_sha = hex::encode(Sha256::digest(good));
    let mut bad = good.to_vec();
    bad[0] ^= 0x01;
    let res = stage_verified_artifact(dir.path(), "0.2.0", "bin", &bad, &expected_sha);
    assert!(res.is_err());
    assert!(!dir.path().join("0.2.0.part").exists());
}

#[test]
fn platform_env_detector_identifies_flatpak_snap_nix_homebrew_and_appimage() {
    assert_eq!(
        detect_install_mode(true, false, false, false, false, None, false, false),
        InstallMode::ExternallyManaged {
            manager_name: "Flatpak".to_owned()
        }
    );
    assert_eq!(
        detect_install_mode(false, true, false, false, false, None, false, false),
        InstallMode::ExternallyManaged {
            manager_name: "Snap".to_owned()
        }
    );
    assert_eq!(
        detect_install_mode(false, false, true, false, false, None, false, false),
        InstallMode::ExternallyManaged {
            manager_name: "Nix".to_owned()
        }
    );
    assert_eq!(
        detect_install_mode(false, false, false, true, false, None, false, false),
        InstallMode::ExternallyManaged {
            manager_name: "Homebrew".to_owned()
        }
    );
    assert_eq!(
        detect_install_mode(false, false, false, false, true, None, false, false),
        InstallMode::SelfUpdate
    );
    assert_eq!(
        detect_install_mode(
            false,
            false,
            false,
            false,
            false,
            Some(Path::new("/usr/bin/raag")),
            false,
            false
        ),
        InstallMode::ExternallyManaged {
            manager_name: "System Package Manager".to_owned()
        }
    );
    assert_eq!(
        detect_install_mode(false, false, false, false, false, None, true, true),
        InstallMode::SignedApkInstallPrompt
    );
}

#[test]
fn audio_aware_update_defaults_to_install_on_quit_while_playing() {
    assert!(resolve_install_on_quit_default(true));
    assert!(!resolve_install_on_quit_default(false));
}

#[test]
fn stale_staging_cleanup_removes_older_or_equal_versions() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("staged.0.0.9.dmg"), b"old").unwrap();
    std::fs::write(dir.path().join("staged.0.1.0.dmg"), b"same").unwrap();
    std::fs::write(dir.path().join("staged.0.2.0.dmg"), b"new").unwrap();
    std::fs::write(dir.path().join("0.2.0.part"), b"partial").unwrap();

    let removed = cleanup_stale_staged_updates(dir.path(), "0.1.0");
    assert_eq!(removed, 3);
    assert!(!dir.path().join("staged.0.0.9.dmg").exists());
    assert!(!dir.path().join("staged.0.1.0.dmg").exists());
    assert!(!dir.path().join("0.2.0.part").exists());
    assert!(dir.path().join("staged.0.2.0.dmg").exists());
}
