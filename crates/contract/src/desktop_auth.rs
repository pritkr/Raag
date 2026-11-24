use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialBackendKind {
    OsKeyring,
    EncryptedFallbackFile,
    MemoryOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthBackendStatus {
    pub backend: CredentialBackendKind,
    pub signed_in: bool,
    pub account_name: Option<String>,
    pub has_sapisid: bool,
    pub has_3papisid: bool,
}
