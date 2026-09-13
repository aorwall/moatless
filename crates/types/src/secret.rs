//! Secret storage settings.
//!
//! The redacting `Debug` impl is what keeps encryption material out of logs, so
//! it travels with the struct.

use std::fmt;

/// Secrets feature bootstrap configuration.
///
/// Implements a custom `Debug` to redact encryption material.
#[derive(Clone)]
pub struct SecretsConfig {
    /// Base64 or raw 32-byte key used by the local envelope cipher.
    ///
    /// A bootstrap value: stays outside the database, used only until the
    /// deployment moves to an external KMS-backed cipher.
    pub encryption_key: Option<String>,
    /// KMS key identifier or alias recorded on encrypted rows.
    pub kms_key_id: Option<String>,
}

#[allow(clippy::missing_fields_in_debug)]
impl fmt::Debug for SecretsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretsConfig")
            .field(
                "encryption_key",
                &self.encryption_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("kms_key_id", &self.kms_key_id)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_config_debug_redacts_encryption_material() {
        let secrets = SecretsConfig {
            encryption_key: Some("real-envelope-key".into()),
            kms_key_id: Some("alias/moatless".into()),
        };
        let debug = format!("{secrets:?}");
        assert!(!debug.contains("real-envelope-key"));
        assert!(debug.contains("[REDACTED]"));
        assert!(debug.contains("alias/moatless")); // kms_key_id is not a secret
    }
}

crate::string_enum!(
    /// Secret value category.
    SecretKind {
        Env => "env",
        ProviderToken => "provider_token",
        RuntimeFile => "runtime_file",
        Deployment => "deployment",
        /// SSH private key. Materialized into the sandbox's `~/.ssh` directory
        /// (never set as an env var) and used transparently for git/ssh over
        /// SSH remotes.
        SshKey => "ssh_key",
        /// Backend-only value (e.g. an OAuth token stashed behind a PAT
        /// override). Never delivered to a sandbox through any channel.
        Internal => "internal",
    }
);

crate::string_enum!(
    /// Where a secret value is stored.
    SecretSource {
        InlineEncrypted => "inline_encrypted",
        ExternalRef => "external_ref",
    }
);
