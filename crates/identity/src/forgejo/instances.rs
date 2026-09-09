//! The OAuth2 application a deployment registers for one Forgejo instance.
//!
//! Forgejo is self-hostable, so "Forgejo" names no single host and no single
//! application. A repository's git URL names the host, and that host selects
//! the application its connect flow runs against.

use std::sync::Arc;

use crate::auth::error::AuthError;
use crate::forgejo::dao::{ForgejoInstanceRecord, PgForgejoInstanceDao};
use crate::secret::service::SecretCipher;
use chrono::{DateTime, Utc};

/// A Forgejo instance's OAuth2 application, opened for use.
///
/// Held only for the duration of one call: [`Self::client_secret`] is
/// plaintext, and nothing caches it.
pub struct ForgejoOAuthApp {
    pub host: String,
    pub client_id: String,
    pub client_secret: String,
}

/// What an admin screen may see about a registered instance.
pub struct RegisteredForgejoInstance {
    pub host: String,
    pub client_id: String,
    pub updated_at: DateTime<Utc>,
}

/// The base URL every REST and OAuth call against `host` is built on.
///
/// HTTPS always: the host is parsed out of a repository's remote URL, which a
/// Workspace controls, and honouring a scheme from there would let one
/// downgrade the connection its own credential travels over.
pub fn base_url(host: &str) -> String {
    format!("https://{host}")
}

/// `host` as it is stored and compared: lowercased, with any scheme, path,
/// query, fragment or trailing dot removed.
///
/// Both writers normalise, because the value is matched against the host
/// [`moatless_types::repository::GitHostRepo::host`] parsed out of a remote
/// URL, which is already lowercase.
pub fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

/// The registered OAuth2 applications, keyed by host.
pub struct ForgejoInstances {
    instances: PgForgejoInstanceDao,
    /// Opens a stored client secret. `None` in a deployment with no encryption
    /// configured, which leaves every registered instance unusable — loudly.
    cipher: Option<Arc<dyn SecretCipher>>,
}

impl ForgejoInstances {
    pub(crate) fn new(
        instances: PgForgejoInstanceDao,
        cipher: Option<Arc<dyn SecretCipher>>,
    ) -> Self {
        Self { instances, cipher }
    }

    /// The application registered for `host`, with its secret opened.
    ///
    /// `Ok(None)` means no application is registered, which leaves a personal
    /// access token as the only way to connect that host. An `Err` means the
    /// registration exists but cannot be read, which is a different thing:
    /// callers must not collapse the two.
    pub async fn app(&self, host: &str) -> Result<Option<ForgejoOAuthApp>, AuthError> {
        let Some(record) = self.instances.find_by_host(&normalize_host(host)).await? else {
            return Ok(None);
        };
        let cipher = self.cipher.as_deref().ok_or_else(|| {
            AuthError::Config(format!(
                "The Forgejo application for '{}' cannot be opened: this deployment configures no \
                 secret encryption key",
                record.host
            ))
        })?;
        let client_secret = cipher
            .decrypt_value(&record.host, &record.client_secret)
            .map_err(|e| {
                AuthError::Config(format!(
                    "Failed to decrypt the Forgejo client secret for '{}': {e}",
                    record.host
                ))
            })?;
        Ok(Some(ForgejoOAuthApp {
            host: record.host,
            client_id: record.client_id,
            client_secret,
        }))
    }

    pub async fn list(&self) -> Result<Vec<RegisteredForgejoInstance>, AuthError> {
        Ok(self
            .instances
            .list()
            .await?
            .into_iter()
            .map(|row| RegisteredForgejoInstance {
                host: row.host,
                client_id: row.client_id,
                updated_at: row.updated_at,
            })
            .collect())
    }

    /// Register `host`'s application, replacing any already registered for it.
    pub async fn register(
        &self,
        host: &str,
        client_id: &str,
        client_secret: &str,
        actor_id: &str,
    ) -> Result<RegisteredForgejoInstance, AuthError> {
        let host = normalize_host(host);
        if host.is_empty() {
            return Err(AuthError::BadRequest(
                "A Forgejo instance needs a hostname".to_string(),
            ));
        }
        let cipher = self.cipher.as_deref().ok_or_else(|| {
            AuthError::Config(
                "A Forgejo application cannot be stored: this deployment configures no secret \
                 encryption key"
                    .to_string(),
            )
        })?;

        // `host` is the ciphertext's additional authenticated data, so this
        // seals without first reading the row: an id read here and written back
        // is stale the moment a second registration for the same host commits
        // between the two, and the ciphertext it stores can never be opened.
        let sealed = cipher.encrypt(&host, client_secret).map_err(|e| {
            AuthError::Config(format!("Failed to encrypt the Forgejo client secret: {e}"))
        })?;

        let updated_at = self
            .instances
            .upsert(
                &ForgejoInstanceRecord {
                    id: format!("fji_{}", uuid::Uuid::new_v4()),
                    host: host.clone(),
                    client_id: client_id.trim().to_string(),
                    client_secret: sealed,
                },
                actor_id,
            )
            .await?;
        Ok(RegisteredForgejoInstance {
            host,
            client_id: client_id.trim().to_string(),
            updated_at,
        })
    }

    /// Remove `host`'s application. Returns whether one was registered.
    ///
    /// The personal access tokens people hold for `host` are untouched: they
    /// authenticate without an application, and removing the row only closes
    /// the connect flow.
    pub async fn remove(&self, host: &str) -> Result<bool, AuthError> {
        self.instances.delete(&normalize_host(host)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_normalizes_to_what_a_remote_url_parses_to() {
        assert_eq!(normalize_host("https://Git.Acme.dev/"), "git.acme.dev");
        assert_eq!(normalize_host(" codeberg.org "), "codeberg.org");
        assert_eq!(normalize_host("git.acme.dev:3000"), "git.acme.dev:3000");
        // A query or a fragment is dropped like a path: what survives has to
        // satisfy `forgejo_instances_host_format_chk`, which admits neither.
        assert_eq!(normalize_host("git.acme.dev?next=/x"), "git.acme.dev");
        assert_eq!(normalize_host("https://git.acme.dev#top"), "git.acme.dev");
    }

    #[test]
    fn a_base_url_is_https_whatever_the_host_was_written_as() {
        // A Workspace controls the remote URL, so honouring a scheme from it
        // would let one downgrade the connection its credential travels over.
        assert_eq!(base_url("git.acme.dev"), "https://git.acme.dev");
    }
}
