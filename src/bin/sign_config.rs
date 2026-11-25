//! Deterministic Ed25519 config artifact signer & verifier (`cargo run --bin sign_config`).
//!
//! Subcommands:
//! - `sign_config --keygen` -> prints `PRIVKEY_HEX=<64-hex>` and `PUBKEY_HEX=<64-hex>`
//! - `sign_config --sign --key-env RESONANCE_CONFIG_ED25519_PRIVKEY_HEX <path>` -> writes `<path>.sig` (128-hex)
//! - `sign_config --verify --pubkey-hex <64-hex> <path> [sig_path]` -> verifies `<path>.sig`

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use resonance_core::innertube::matrix::ClientMatrix;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        return ExitCode::SUCCESS;
    }

    if args.iter().any(|a| a == "--keygen") {
        let mut seed = [0u8; 32];
        rand::fill(&mut seed);
        let signing = SigningKey::from_bytes(&seed);
        let verifying = signing.verifying_key();
        println!("PRIVKEY_HEX={}", hex::encode(signing.to_bytes()));
        println!("PUBKEY_HEX={}", hex::encode(verifying.to_bytes()));
        return ExitCode::SUCCESS;
    }

    if args.iter().any(|a| a == "--verify") {
        let pubkey_hex = arg_value(&args, "--pubkey-hex")
            .or_else(|| std::env::var("RESONANCE_CONFIG_ED25519_PUBKEY_HEX").ok());
        let Some(pubkey_hex) = pubkey_hex else {
            eprintln!("error: --pubkey-hex or RESONANCE_CONFIG_ED25519_PUBKEY_HEX is required");
            return ExitCode::from(2);
        };
        let positional = positional_args(&args, &["--pubkey-hex"]);
        let Some(target) = positional.first() else {
            eprintln!("error: missing <artifact_path>");
            return ExitCode::from(2);
        };
        let sig_path = positional
            .get(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("{target}.sig")));
        match verify_file(Path::new(target), &sig_path, &pubkey_hex) {
            Ok(()) => {
                println!("OK: verified {}", target);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("VERIFY_FAILED: {e}");
                return ExitCode::from(1);
            }
        }
    }

    // Default or explicit `--sign`
    let key_env = arg_value(&args, "--key-env")
        .unwrap_or_else(|| "RESONANCE_CONFIG_ED25519_PRIVKEY_HEX".to_owned());
    let Ok(priv_hex) = std::env::var(&key_env) else {
        eprintln!("error: environment variable {key_env} is not set");
        return ExitCode::from(2);
    };
    let positional = positional_args(&args, &["--key-env"]);
    if positional.is_empty() {
        eprintln!("error: missing <artifact_path>");
        return ExitCode::from(2);
    }

    for target in positional {
        let path = Path::new(&target);
        match sign_file(path, &priv_hex) {
            Ok(sig_path) => println!("SIGNED: {} -> {}", path.display(), sig_path.display()),
            Err(e) => {
                eprintln!("SIGN_FAILED ({}): {e}", path.display());
                return ExitCode::from(1);
            }
        }
    }

    ExitCode::SUCCESS
}

fn sign_file(path: &Path, priv_hex: &str) -> Result<PathBuf, String> {
    let key_bytes =
        hex::decode(priv_hex.trim()).map_err(|e| format!("invalid hex private key: {e}"))?;
    let key_arr: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| "Ed25519 private key must be 32 bytes (64 hex chars)".to_owned())?;
    let signing_key = SigningKey::from_bytes(&key_arr);

    let payload = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    validate_matrix_invariants_if_applicable(path, &payload)?;

    let sig = signing_key.sign(&payload);
    let sig_hex = hex::encode(sig.to_bytes());
    let sig_path = PathBuf::from(format!("{}.sig", path.display()));
    fs::write(&sig_path, sig_hex).map_err(|e| format!("write {}: {e}", sig_path.display()))?;
    Ok(sig_path)
}

fn verify_file(path: &Path, sig_path: &Path, pub_hex: &str) -> Result<(), String> {
    let pub_bytes =
        hex::decode(pub_hex.trim()).map_err(|e| format!("invalid hex public key: {e}"))?;
    let pub_arr: [u8; 32] = pub_bytes
        .try_into()
        .map_err(|_| "Ed25519 public key must be 32 bytes (64 hex chars)".to_owned())?;
    let verifying_key =
        VerifyingKey::from_bytes(&pub_arr).map_err(|e| format!("invalid public key: {e}"))?;

    let payload = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    validate_matrix_invariants_if_applicable(path, &payload)?;

    let sig_raw = fs::read(sig_path).map_err(|e| format!("read {}: {e}", sig_path.display()))?;
    let sig_bytes: [u8; 64] = if sig_raw.len() == 64 {
        sig_raw
            .try_into()
            .map_err(|_| "invalid 64-byte signature".to_owned())?
    } else {
        let s = std::str::from_utf8(&sig_raw)
            .map_err(|e| format!("non-UTF8 signature file: {e}"))?
            .trim();
        let decoded = hex::decode(s).map_err(|e| format!("invalid hex signature: {e}"))?;
        decoded
            .try_into()
            .map_err(|_| "Ed25519 signature must be 64 bytes".to_owned())?
    };
    let signature = Signature::from_bytes(&sig_bytes);
    verifying_key
        .verify(&payload, &signature)
        .map_err(|e| format!("signature mismatch: {e}"))
}

fn validate_matrix_invariants_if_applicable(path: &Path, payload: &[u8]) -> Result<(), String> {
    let is_matrix = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.contains("client_matrix"));
    if !is_matrix {
        return Ok(());
    }
    let matrix: ClientMatrix =
        serde_json::from_slice(payload).map_err(|e| format!("invalid ClientMatrix JSON: {e}"))?;
    if matrix.schema_version != 1 {
        return Err(format!(
            "unsupported schema_version {}",
            matrix.schema_version
        ));
    }
    if !matrix.clients.iter().any(|c| c.enabled) {
        return Err("refusing to sign matrix with 0 enabled clients".to_owned());
    }
    for entry in &matrix.clients {
        if matches!(entry.kind.as_str(), "android_vr" | "ios")
            && (entry.auth_mode != "anonymous_only" || entry.probe_pass_cookies_on)
        {
            return Err(format!(
                "security invariant violated: {} must remain anonymous_only without cookies",
                entry.kind
            ));
        }
    }
    Ok(())
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

fn positional_args(args: &[String], value_flags: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_next = false;
    for a in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if value_flags.contains(&a.as_str()) {
            skip_next = true;
            continue;
        }
        if a.starts_with("--") {
            continue;
        }
        out.push(a.clone());
    }
    out
}

fn print_usage() {
    println!(
        "Usage:\n  \
         sign_config --keygen\n  \
         sign_config --sign [--key-env ENV] <file>...\n  \
         sign_config --verify [--pubkey-hex HEX] <file> [sig_file]"
    );
}
