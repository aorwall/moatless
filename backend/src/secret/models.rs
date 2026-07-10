//! Secret domain types and validation helpers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Scope type for application-managed secrets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecretScopeType {
    Global,
    Team,
    User,
}

impl SecretScopeType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Team => "team",
            Self::User => "user",
        }
    }
}

impl std::fmt::Display for SecretScopeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for SecretScopeType {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "global" => Ok(Self::Global),
            "team" => Ok(Self::Team),
            "user" => Ok(Self::User),
            other => Err(format!("unknown secret scope type: {other}")),
        }
    }
}

/// Secret value category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    Env,
    ProviderToken,
    RuntimeFile,
    Deployment,
    /// SSH private key. Materialized into the sandbox's `~/.ssh` directory
    /// (never set as an env var) and used transparently for git/ssh over SSH
    /// remotes.
    SshKey,
}

impl SecretKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::ProviderToken => "provider_token",
            Self::RuntimeFile => "runtime_file",
            Self::Deployment => "deployment",
            Self::SshKey => "ssh_key",
        }
    }
}

impl std::fmt::Display for SecretKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for SecretKind {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "env" => Ok(Self::Env),
            "provider_token" => Ok(Self::ProviderToken),
            "runtime_file" => Ok(Self::RuntimeFile),
            "deployment" => Ok(Self::Deployment),
            "ssh_key" => Ok(Self::SshKey),
            other => Err(format!("unknown secret kind: {other}")),
        }
    }
}

/// Where a secret value is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecretSource {
    InlineEncrypted,
    ExternalRef,
}

/// Persisted secret row. The plaintext value is intentionally absent.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct SecretRecord {
    pub id: String,
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub key: String,
    pub kind: String,
    pub provider: Option<String>,
    pub value_ciphertext: Option<Vec<u8>>,
    pub encryption_nonce: Option<Vec<u8>>,
    pub encrypted_data_key: Option<Vec<u8>>,
    pub external_ref: Option<String>,
    pub kms_key_id: Option<String>,
    pub encryption_version: i32,
    pub created_by: String,
    pub updated_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub version: i32,
}

impl SecretRecord {
    pub fn source(&self) -> SecretSource {
        if self.external_ref.is_some() {
            SecretSource::ExternalRef
        } else {
            SecretSource::InlineEncrypted
        }
    }
}

/// Resolved plaintext secret for internal runtime consumers only.
#[derive(Debug, Clone)]
pub struct ResolvedSecret {
    pub key: String,
    pub value: String,
    pub provider: Option<String>,
    /// Raw `kind` string from the record (e.g. `"env"`, `"ssh_key"`). Lets
    /// callers route `ssh_key` secrets to `~/.ssh` instead of the env-var map.
    pub kind: String,
}

/// Storage payload for an encrypted inline secret.
#[derive(Debug, Clone)]
pub struct EncryptedSecretValue {
    pub value_ciphertext: Vec<u8>,
    pub encryption_nonce: Vec<u8>,
    pub encrypted_data_key: Vec<u8>,
    pub kms_key_id: String,
    pub encryption_version: i32,
}

/// Key names controlled by the platform/runtime.
pub const RESERVED_SECRET_KEYS: &[&str] = &[
    "TASK_ID",
    "BACKEND_GRPC_URL",
    "BACKEND_API_URL",
    "USER_AUTH_TOKEN",
    "TASK_TOKEN_SECRET",
    "SESSION_JWT_SECRET",
    "DATABASE_URL",
    "KUBERNETES_SERVICE_HOST",
];

/// Env-style key validation: `^[A-Z_][A-Z0-9_]*$`.
pub fn is_valid_secret_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_uppercase()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_ascii_uppercase() || c.is_ascii_digit())
}

pub fn is_reserved_key(key: &str) -> bool {
    RESERVED_SECRET_KEYS.contains(&key)
}

/// Detect keys that should be moved from repository env vars into secrets.
pub fn looks_like_secret_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    is_reserved_key(&upper)
        || upper.contains("SECRET")
        || upper.contains("TOKEN")
        || upper.contains("PASSWORD")
        || upper.contains("PRIVATE_KEY")
        || upper.ends_with("_API_KEY")
        || upper.ends_with("_KEY")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_secret_key_format() {
        assert!(is_valid_secret_key("STRIPE_SECRET_KEY"));
        assert!(is_valid_secret_key("_TOKEN"));
        assert!(is_valid_secret_key("A1"));
        assert!(!is_valid_secret_key(""));
        assert!(!is_valid_secret_key("stripe_key"));
        assert!(!is_valid_secret_key("1TOKEN"));
        assert!(!is_valid_secret_key("BAD-KEY"));
    }

    #[test]
    fn detects_likely_secret_keys() {
        assert!(looks_like_secret_key("STRIPE_SECRET_KEY"));
        assert!(looks_like_secret_key("OPENAI_API_KEY"));
        assert!(looks_like_secret_key("TASK_TOKEN_SECRET"));
        assert!(!looks_like_secret_key("NODE_ENV"));
        assert!(!looks_like_secret_key("PUBLIC_URL"));
    }
}
