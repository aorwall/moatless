//! Durable storage for GitHub App registrations.
//!
//! A registration is an administered record: an admin creates it, rotates its
//! signing key, and removes it, and every backend serving the installation sees
//! the result from its next use onwards. The App used for signing in is not
//! stored here — it stays deployment-configured, so nothing done through the UI
//! can make the UI unreachable.
//!
//! The signing key is envelope-encrypted on the record itself, under the
//! record's id, using the same cipher the `secrets` table uses. It is never
//! returned by a read path; what callers get instead is a fingerprint, which is
//! enough to tell two keys apart and reveals neither.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::AssertSqlSafe;

use crate::auth::error::AuthError;
use crate::db::DbPool;
use crate::secret::models::EncryptedSecretValue;
use crate::secret::service::SecretCipher;

/// Name of the one-time carry-over of `GITHUB_APPS_JSON` into durable storage.
pub const ENV_CARRY_OVER: &str = "github_apps_json_carry_over";

/// A registered GitHub App.
#[derive(Clone)]
pub struct GitHubAppRecord {
    pub id: String,
    /// Short stable name the App is referenced by. Case-sensitive: bot users
    /// already store keys verbatim, so normalising would orphan them.
    pub app_key: String,
    /// GitHub's own numeric id for the App.
    pub app_id: u64,
    /// Display name, resolved from GitHub when the App was proved.
    pub name: String,
    pub slug: String,
    pub bot_login: String,
    /// Non-reversing digest of the signing key, for telling two keys apart in
    /// a surface that must never show either.
    pub key_fingerprint: String,
    pub default_installation_id: Option<u64>,
    /// Bumped on every write. Obtained GitHub tokens are held against this, so
    /// editing one App discards only that App's tokens.
    pub revision: i32,
    pub updated_at: DateTime<Utc>,
    private_key: EncryptedSecretValue,
}

#[allow(clippy::missing_fields_in_debug)]
// reason: the encrypted key is deliberately absent, not omitted by oversight.
impl std::fmt::Debug for GitHubAppRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubAppRecord")
            .field("app_key", &self.app_key)
            .field("app_id", &self.app_id)
            .field("name", &self.name)
            .field("slug", &self.slug)
            .field("bot_login", &self.bot_login)
            .field("key_fingerprint", &self.key_fingerprint)
            .field("default_installation_id", &self.default_installation_id)
            .field("revision", &self.revision)
            .finish()
    }
}

/// Everything about an App that comes from outside storage: what the admin
/// supplied, plus what GitHub confirmed when the key was proved.
pub struct NewGitHubApp {
    pub app_key: String,
    pub app_id: u64,
    pub name: String,
    pub slug: String,
    pub bot_login: String,
    pub private_key_pem: String,
    pub default_installation_id: Option<u64>,
}

impl GitHubAppRecord {
    /// Encrypt a proved App into a storable record.
    ///
    /// The record id is generated here because it is the additional
    /// authenticated data for the key ciphertext — a key encrypted for one
    /// record cannot be replayed onto another.
    pub fn seal(
        app: NewGitHubApp,
        cipher: &dyn SecretCipher,
        existing_id: Option<String>,
    ) -> Result<Self, AuthError> {
        let id = existing_id.unwrap_or_else(|| format!("gha_{}", uuid::Uuid::new_v4()));
        let private_key = cipher
            .encrypt(&id, &app.private_key_pem)
            .map_err(|e| AuthError::Config(format!("Failed to encrypt the signing key: {e}")))?;
        Ok(Self {
            id,
            app_key: app.app_key,
            app_id: app.app_id,
            name: app.name,
            slug: app.slug,
            bot_login: app.bot_login,
            key_fingerprint: fingerprint(&app.private_key_pem),
            default_installation_id: app.default_installation_id,
            revision: 1,
            updated_at: Utc::now(),
            private_key,
        })
    }

    /// Recover the signing key. The only path that yields it, and it never
    /// leaves the backend.
    pub fn private_key_pem(&self, cipher: &dyn SecretCipher) -> Result<String, AuthError> {
        cipher
            .decrypt_value(&self.id, &self.private_key)
            .map_err(|e| {
                AuthError::Config(format!(
                    "Failed to decrypt the signing key for App '{}': {e}",
                    self.app_key
                ))
            })
    }
}

/// A short, non-reversing digest of a signing key.
fn fingerprint(private_key_pem: &str) -> String {
    let digest = Sha256::digest(private_key_pem.trim().as_bytes());
    hex::encode(&digest[..8])
}

/// One thing an admin did to an App, and whether it worked.
pub struct GitHubAppAuditEntry<'a> {
    pub github_app_id: Option<&'a str>,
    pub app_key: &'a str,
    pub actor_id: &'a str,
    pub action: &'a str,
    pub result: &'a str,
    pub detail: Option<&'a str>,
}

/// Durable state behind GitHub App administration.
#[async_trait]
pub trait GitHubAppStore: Send + Sync {
    async fn list(&self) -> Result<Vec<GitHubAppRecord>, AuthError>;

    async fn find_by_key(&self, app_key: &str) -> Result<Option<GitHubAppRecord>, AuthError>;

    /// Store a newly proved App. A key already in use is a conflict, never an
    /// overwrite — overwriting would silently repoint every bot user that
    /// names the key.
    async fn insert(&self, record: &GitHubAppRecord, actor_id: &str) -> Result<(), AuthError>;

    /// Replace an App's signing key and refreshed identity, bumping its
    /// revision. `false` when no App holds that key.
    async fn replace(&self, record: &GitHubAppRecord, actor_id: &str) -> Result<bool, AuthError>;

    /// Remove an App. `false` when no App holds that key.
    async fn delete(&self, app_key: &str) -> Result<bool, AuthError>;

    /// Store carried-over Apps and record the carry-over as done, together.
    ///
    /// `Ok(None)` means another backend already did it, so several backends
    /// starting together produce one outcome.
    ///
    /// Both parts commit or neither does. Recording completion first would let
    /// a crash, a cancellation or a transient database failure partway through
    /// leave the marker claimed and the Apps unwritten — the environment
    /// variable still set, and nothing that would ever retry it.
    ///
    /// `Ok(Some(keys))` names the Apps that were *not* written because that key
    /// was already registered. They are skipped rather than overwritten, and
    /// named rather than dropped in silence.
    async fn carry_over(
        &self,
        marker: &str,
        detail: &str,
        records: &[GitHubAppRecord],
        actor_id: &str,
    ) -> Result<Option<Vec<String>>, AuthError>;

    async fn record_audit(&self, entry: GitHubAppAuditEntry<'_>) -> Result<(), AuthError>;
}

#[derive(sqlx::FromRow)]
struct GitHubAppRow {
    id: String,
    app_key: String,
    app_id: i64,
    name: String,
    slug: String,
    bot_login: String,
    private_key_ciphertext: Vec<u8>,
    private_key_nonce: Vec<u8>,
    private_key_data_key: Vec<u8>,
    kms_key_id: String,
    encryption_version: i32,
    key_fingerprint: String,
    default_installation_id: Option<i64>,
    revision: i32,
    updated_at: DateTime<Utc>,
}

impl From<GitHubAppRow> for GitHubAppRecord {
    fn from(row: GitHubAppRow) -> Self {
        Self {
            id: row.id,
            app_key: row.app_key,
            app_id: row.app_id.unsigned_abs(),
            name: row.name,
            slug: row.slug,
            bot_login: row.bot_login,
            key_fingerprint: row.key_fingerprint,
            default_installation_id: row.default_installation_id.map(i64::unsigned_abs),
            revision: row.revision,
            updated_at: row.updated_at,
            private_key: EncryptedSecretValue {
                value_ciphertext: row.private_key_ciphertext,
                encryption_nonce: row.private_key_nonce,
                encrypted_data_key: row.private_key_data_key,
                kms_key_id: row.kms_key_id,
                encryption_version: row.encryption_version,
            },
        }
    }
}

const SELECT_COLUMNS: &str = r#"
    id, app_key, app_id, name, slug, bot_login,
    private_key_ciphertext, private_key_nonce, private_key_data_key,
    kms_key_id, encryption_version, key_fingerprint,
    default_installation_id, revision, updated_at
"#;

/// Shared by `insert` and `carry_over` so the two cannot drift. Both leave a
/// taken key alone rather than overwriting it: an overwrite would silently
/// repoint every bot user that names the key.
const INSERT_APP: &str = r#"
    INSERT INTO github_apps (
        id, app_key, app_id, name, slug, bot_login,
        private_key_ciphertext, private_key_nonce, private_key_data_key,
        kms_key_id, encryption_version, key_fingerprint,
        default_installation_id, created_by, updated_by, revision
    )
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $14, 1)
    ON CONFLICT (app_key) DO NOTHING
"#;

pub struct PgGitHubAppStore(pub DbPool);

#[async_trait]
impl GitHubAppStore for PgGitHubAppStore {
    async fn list(&self) -> Result<Vec<GitHubAppRecord>, AuthError> {
        let rows: Vec<GitHubAppRow> = sqlx::query_as(AssertSqlSafe(format!(
            "SELECT {SELECT_COLUMNS} FROM github_apps ORDER BY app_key"
        )))
        .fetch_all(&self.0)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn find_by_key(&self, app_key: &str) -> Result<Option<GitHubAppRecord>, AuthError> {
        let row: Option<GitHubAppRow> = sqlx::query_as(AssertSqlSafe(format!(
            "SELECT {SELECT_COLUMNS} FROM github_apps WHERE app_key = $1"
        )))
        .bind(app_key)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.map(Into::into))
    }

    async fn insert(&self, record: &GitHubAppRecord, actor_id: &str) -> Result<(), AuthError> {
        let inserted = sqlx::query(INSERT_APP)
            .bind(&record.id)
        .bind(&record.app_key)
        .bind(i64::try_from(record.app_id).unwrap_or(i64::MAX))
        .bind(&record.name)
        .bind(&record.slug)
        .bind(&record.bot_login)
        .bind(&record.private_key.value_ciphertext)
        .bind(&record.private_key.encryption_nonce)
        .bind(&record.private_key.encrypted_data_key)
        .bind(&record.private_key.kms_key_id)
        .bind(record.private_key.encryption_version)
        .bind(&record.key_fingerprint)
        .bind(record.default_installation_id.and_then(|v| i64::try_from(v).ok()))
        .bind(actor_id)
        .execute(&self.0)
        .await?;

        if inserted.rows_affected() == 0 {
            return Err(AuthError::Conflict(format!(
                "A GitHub App is already registered as '{}'",
                record.app_key
            )));
        }
        Ok(())
    }

    async fn replace(&self, record: &GitHubAppRecord, actor_id: &str) -> Result<bool, AuthError> {
        let updated = sqlx::query(
            r#"UPDATE github_apps SET
                   app_id = $2,
                   name = $3,
                   slug = $4,
                   bot_login = $5,
                   private_key_ciphertext = $6,
                   private_key_nonce = $7,
                   private_key_data_key = $8,
                   kms_key_id = $9,
                   encryption_version = $10,
                   key_fingerprint = $11,
                   default_installation_id = $12,
                   updated_by = $13,
                   updated_at = now(),
                   revision = revision + 1
               WHERE app_key = $1"#,
        )
        .bind(&record.app_key)
        .bind(i64::try_from(record.app_id).unwrap_or(i64::MAX))
        .bind(&record.name)
        .bind(&record.slug)
        .bind(&record.bot_login)
        .bind(&record.private_key.value_ciphertext)
        .bind(&record.private_key.encryption_nonce)
        .bind(&record.private_key.encrypted_data_key)
        .bind(&record.private_key.kms_key_id)
        .bind(record.private_key.encryption_version)
        .bind(&record.key_fingerprint)
        .bind(record.default_installation_id.and_then(|v| i64::try_from(v).ok()))
        .bind(actor_id)
        .execute(&self.0)
        .await?;
        Ok(updated.rows_affected() > 0)
    }

    async fn delete(&self, app_key: &str) -> Result<bool, AuthError> {
        let deleted = sqlx::query("DELETE FROM github_apps WHERE app_key = $1")
            .bind(app_key)
            .execute(&self.0)
            .await?;
        Ok(deleted.rows_affected() > 0)
    }

    async fn carry_over(
        &self,
        marker: &str,
        detail: &str,
        records: &[GitHubAppRecord],
        actor_id: &str,
    ) -> Result<Option<Vec<String>>, AuthError> {
        let mut tx = self.0.begin().await?;

        // Claimed first *within the transaction*, so a second backend blocks on
        // this row until the first commits and then sees it taken. Nothing is
        // visible to anyone until the Apps are written alongside it.
        let claimed = sqlx::query(
            r#"INSERT INTO github_app_migrations (name, detail)
               VALUES ($1, $2)
               ON CONFLICT (name) DO NOTHING"#,
        )
        .bind(marker)
        .bind(detail)
        .execute(&mut *tx)
        .await?;
        if claimed.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(None);
        }

        let mut skipped = Vec::new();
        for record in records {
            let inserted = sqlx::query(INSERT_APP)
                .bind(&record.id)
                .bind(&record.app_key)
                .bind(i64::try_from(record.app_id).unwrap_or(i64::MAX))
                .bind(&record.name)
                .bind(&record.slug)
                .bind(&record.bot_login)
                .bind(&record.private_key.value_ciphertext)
                .bind(&record.private_key.encryption_nonce)
                .bind(&record.private_key.encrypted_data_key)
                .bind(&record.private_key.kms_key_id)
                .bind(record.private_key.encryption_version)
                .bind(&record.key_fingerprint)
                .bind(
                    record
                        .default_installation_id
                        .and_then(|v| i64::try_from(v).ok()),
                )
                .bind(actor_id)
                .execute(&mut *tx)
                .await?;
            if inserted.rows_affected() == 0 {
                skipped.push(record.app_key.clone());
            }
        }

        tx.commit().await?;
        Ok(Some(skipped))
    }

    async fn record_audit(&self, entry: GitHubAppAuditEntry<'_>) -> Result<(), AuthError> {
        sqlx::query(
            r#"INSERT INTO github_app_audit_events
                   (id, github_app_id, app_key, actor_id, action, result, detail)
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(entry.github_app_id)
        .bind(entry.app_key)
        .bind(entry.actor_id)
        .bind(entry.action)
        .bind(entry.result)
        .bind(entry.detail)
        .execute(&self.0)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
pub mod test_support {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::{AuthError, GitHubAppAuditEntry, GitHubAppRecord, GitHubAppStore};
    use async_trait::async_trait;

    /// In-memory store for registry tests, which need to change what durable
    /// state says between two lookups.
    #[derive(Default)]
    pub struct InMemoryGitHubAppStore {
        apps: Mutex<HashMap<String, GitHubAppRecord>>,
        migrations: Mutex<Vec<String>>,
        pub reads: Mutex<usize>,
        pub fail_reads: Mutex<bool>,
        pub fail_writes: Mutex<bool>,
    }

    impl InMemoryGitHubAppStore {
        pub fn put(&self, record: GitHubAppRecord) {
            self.apps
                .lock()
                .expect("lock")
                .insert(record.app_key.clone(), record);
        }

        /// Has the one-time carry-over been recorded as done?
        pub fn migration_claimed(&self, marker: &str) -> bool {
            self.migrations.lock().expect("lock").iter().any(|m| m == marker)
        }

        /// Simulate an edit landing between two lookups.
        pub fn bump_revision(&self, app_key: &str) {
            if let Some(record) = self.apps.lock().expect("lock").get_mut(app_key) {
                record.revision += 1;
            }
        }

        pub fn read_count(&self) -> usize {
            *self.reads.lock().expect("lock")
        }
    }

    #[async_trait]
    impl GitHubAppStore for InMemoryGitHubAppStore {
        async fn list(&self) -> Result<Vec<GitHubAppRecord>, AuthError> {
            if *self.fail_reads.lock().expect("lock") {
                return Err(AuthError::Internal("store unavailable".into()));
            }
            let mut apps: Vec<GitHubAppRecord> =
                self.apps.lock().expect("lock").values().cloned().collect();
            apps.sort_by(|a, b| a.app_key.cmp(&b.app_key));
            Ok(apps)
        }

        async fn find_by_key(&self, app_key: &str) -> Result<Option<GitHubAppRecord>, AuthError> {
            *self.reads.lock().expect("lock") += 1;
            if *self.fail_reads.lock().expect("lock") {
                return Err(AuthError::Internal("store unavailable".into()));
            }
            Ok(self.apps.lock().expect("lock").get(app_key).cloned())
        }

        async fn insert(&self, record: &GitHubAppRecord, _actor_id: &str) -> Result<(), AuthError> {
            let mut apps = self.apps.lock().expect("lock");
            if apps.contains_key(&record.app_key) {
                return Err(AuthError::Conflict(format!(
                    "A GitHub App is already registered as '{}'",
                    record.app_key
                )));
            }
            apps.insert(record.app_key.clone(), record.clone());
            Ok(())
        }

        async fn replace(
            &self,
            record: &GitHubAppRecord,
            _actor_id: &str,
        ) -> Result<bool, AuthError> {
            let mut apps = self.apps.lock().expect("lock");
            let Some(existing) = apps.get(&record.app_key) else {
                return Ok(false);
            };
            let mut next = record.clone();
            next.revision = existing.revision + 1;
            apps.insert(record.app_key.clone(), next);
            Ok(true)
        }

        async fn delete(&self, app_key: &str) -> Result<bool, AuthError> {
            Ok(self.apps.lock().expect("lock").remove(app_key).is_some())
        }

        async fn carry_over(
            &self,
            marker: &str,
            _detail: &str,
            records: &[GitHubAppRecord],
            _actor_id: &str,
        ) -> Result<Option<Vec<String>>, AuthError> {
            // Held across the whole operation, which is what the Postgres
            // store gets from doing all of this in one transaction.
            let mut migrations = self.migrations.lock().expect("lock");
            if migrations.iter().any(|m| m == marker) {
                return Ok(None);
            }
            if *self.fail_writes.lock().expect("lock") {
                // Nothing written, and crucially the marker is not recorded —
                // the next start must try again.
                return Err(AuthError::Internal("store unavailable".into()));
            }
            migrations.push(marker.to_string());

            let mut apps = self.apps.lock().expect("lock");
            let mut skipped = Vec::new();
            for record in records {
                if apps.contains_key(&record.app_key) {
                    skipped.push(record.app_key.clone());
                    continue;
                }
                apps.insert(record.app_key.clone(), record.clone());
            }
            Ok(Some(skipped))
        }

        async fn record_audit(&self, _entry: GitHubAppAuditEntry<'_>) -> Result<(), AuthError> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SecretsConfig;
    use crate::secret::service::LocalEnvelopeSecretCipher;

    fn cipher() -> std::sync::Arc<dyn SecretCipher> {
        LocalEnvelopeSecretCipher::from_config(&SecretsConfig {
            encryption_key: Some("0123456789abcdef0123456789abcdef".into()),
            kms_key_id: None,
        })
        .expect("cipher")
    }

    fn new_app(app_key: &str, pem: &str) -> NewGitHubApp {
        NewGitHubApp {
            app_key: app_key.to_string(),
            app_id: 42,
            name: "Reviewer".into(),
            slug: "reviewer".into(),
            bot_login: "reviewer[bot]".into(),
            private_key_pem: pem.to_string(),
            default_installation_id: None,
        }
    }

    #[test]
    fn a_sealed_key_comes_back_intact() {
        let cipher = cipher();
        let record =
            GitHubAppRecord::seal(new_app("reviewer", "-----PEM-----"), cipher.as_ref(), None)
                .expect("seal");
        assert_eq!(
            record.private_key_pem(cipher.as_ref()).expect("open"),
            "-----PEM-----"
        );
    }

    #[test]
    fn a_key_sealed_for_one_record_cannot_be_opened_as_another() {
        // The record id is the additional authenticated data, so moving
        // ciphertext between rows must fail rather than decrypt.
        let cipher = cipher();
        let mut record =
            GitHubAppRecord::seal(new_app("reviewer", "-----PEM-----"), cipher.as_ref(), None)
                .expect("seal");
        record.id = "gha_someone_else".into();
        assert!(record.private_key_pem(cipher.as_ref()).is_err());
    }

    #[test]
    fn the_fingerprint_distinguishes_keys_without_revealing_them() {
        let one = fingerprint("-----KEY ONE-----");
        let two = fingerprint("-----KEY TWO-----");
        assert_ne!(one, two);
        assert_eq!(one.len(), 16);
        assert!(!one.contains("KEY"));
    }

    #[test]
    fn trailing_whitespace_does_not_change_a_fingerprint() {
        // A key pasted into a form usually gains a newline; two admins pasting
        // the same key should not see two different fingerprints.
        assert_eq!(fingerprint("-----PEM-----"), fingerprint("-----PEM-----\n"));
    }

    #[test]
    fn the_debug_output_carries_no_key_material() {
        let cipher = cipher();
        let record =
            GitHubAppRecord::seal(new_app("reviewer", "SUPERSECRET"), cipher.as_ref(), None)
                .expect("seal");
        let debug = format!("{record:?}");
        assert!(!debug.contains("SUPERSECRET"));
        assert!(debug.contains("reviewer"));
    }
}
