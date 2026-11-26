//! Fail-closed Ed25519 manifest signature verification & SHA-256 artifact verification.

use std::path::Path;

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

/// Verifies `manifest_bytes` against `sig_bytes` (64-byte raw Ed25519 signature)
/// using `verify_strict` to prevent malleable signatures.
pub fn verify_manifest_ed25519(
    manifest_bytes: &[u8],
    sig_bytes: &[u8],
    pubkey_bytes: &[u8; 32],
) -> Result<(), String> {
    let key = VerifyingKey::from_bytes(pubkey_bytes)
        .map_err(|e| format!("invalid updater Ed25519 public key: {e}"))?;
    let sig_arr: [u8; 64] = sig_bytes.try_into().map_err(|_| {
        format!(
            "invalid signature length: {} bytes (expected 64)",
            sig_bytes.len()
        )
    })?;
    let sig = Signature::from_bytes(&sig_arr);
    key.verify_strict(manifest_bytes, &sig)
        .map_err(|e| format!("Ed25519 manifest signature verification failed: {e}"))
}

/// Verifies an in-memory buffer's SHA-256 hex digest (case-insensitive).
pub fn verify_sha256_bytes(bytes: &[u8], expected_hex: &str) -> Result<(), String> {
    let actual = hex::encode(Sha256::digest(bytes));
    let expected = expected_hex.trim().to_ascii_lowercase();
    if actual != expected {
        return Err(format!(
            "SHA-256 mismatch: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

/// Verifies a file on disk against its expected SHA-256 hex digest.
pub fn verify_sha256_file(path: &Path, expected_hex: &str) -> Result<(), String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("failed to read artifact {}: {e}", path.display()))?;
    verify_sha256_bytes(&bytes, expected_hex)
}
