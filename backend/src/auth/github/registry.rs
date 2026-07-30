//! Lookup for configured GitHub Apps, keyed by App key.
//!
//! Two kinds of App answer the same lookup:
//!
//! - **The login App**, configured by the deployment (`GITHUB_APP_*`) and
//!   registered under [`GitHubAppService::DEFAULT_KEY`]. It is start-up state
//!   and cannot be changed from any screen, so that nothing done through the UI
//!   can make the UI unreachable.
//! - **Bot Apps**, registered by admins and held in durable storage. These are
//!   read at the moment they are used, never cached as a list, so an App
//!   registered on one backend is usable from every backend at once.
//!
//! What an App *is* is never cached. What an App *has obtained from GitHub* —
//! minted tokens, resolved installation ids, its bot identity — is, and it is
//! held against the specific stored record it came from. Editing one App
//! discards that App's obtained tokens and nobody else's.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::auth::error::AuthError;
use crate::auth::github::service::GitHubAppService;
use crate::auth::github::dao::{
    GitHubAppDao, GitHubAppRecord, NewGitHubApp, ENV_CARRY_OVER,
};
use crate::config::GitHubAppConfig;
use crate::secret::service::SecretCipher;

/// A live App service, and which stored record it was built from.
///
/// The record id is held alongside the revision because the revision alone
/// does not identify a record: removing an App and registering a new one under
/// the same key produces a fresh record that also starts at revision 1, and a
/// backend holding the old one would go on minting tokens with the deleted
/// signing key.
struct CachedApp {
    record_id: String,
    revision: i32,
    service: Arc<GitHubAppService>,
}

/// Lookup for every GitHub App this installation can use.
#[derive(Default)]
pub struct GitHubAppRegistry {
    /// The deployment-configured login App, if configured.
    login_app: Option<Arc<GitHubAppService>>,
    /// Durable state for admin-registered Apps. Absent in minimal wiring
    /// (no database), where only the login App exists.
    store: Option<Arc<dyn GitHubAppDao>>,
    /// Needed to open a stored signing key. Absent when the installation has
    /// no encryption configured, which makes stored Apps unusable — loudly.
    cipher: Option<Arc<dyn SecretCipher>>,
    /// Services built from stored records, held against the record they came
    /// from.
    cache: RwLock<HashMap<String, CachedApp>>,
}

impl GitHubAppRegistry {
    /// Build the registry.
    ///
    /// The login App is supplied by the caller because it is deployment
    /// configuration and cannot change afterwards. Everything else comes from
    /// `store` at the moment it is asked for.
    pub fn new(
        login_app: Option<Arc<GitHubAppService>>,
        store: Option<Arc<dyn GitHubAppDao>>,
        cipher: Option<Arc<dyn SecretCipher>>,
    ) -> Self {
        Self {
            login_app,
            store,
            cipher,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Look up an App by its key.
    ///
    /// `Ok(None)` means no App is registered under that key. An `Err` means
    /// durable state could not be read, which is a different thing entirely:
    /// one says an admin should register something, the other says the
    /// installation is unhealthy. Callers must not collapse the two.
    pub async fn get(&self, key: &str) -> Result<Option<Arc<GitHubAppService>>, AuthError> {
        if let Some(login_app) = &self.login_app {
            if login_app.key() == key {
                return Ok(Some(Arc::clone(login_app)));
            }
        }

        let Some(store) = &self.store else {
            return Ok(None);
        };
        let Some(record) = store.find_by_key(key).await? else {
            // A stale cache entry for an App that has since been removed must
            // not keep answering.
            self.cache.write().await.remove(key);
            return Ok(None);
        };

        if let Some(cached) = self.cache.read().await.get(key) {
            if cached.record_id == record.id && cached.revision == record.revision {
                return Ok(Some(Arc::clone(&cached.service)));
            }
        }

        let service = Arc::new(self.build(&record)?);
        self.cache.write().await.insert(
            key.to_string(),
            CachedApp {
                record_id: record.id.clone(),
                revision: record.revision,
                service: Arc::clone(&service),
            },
        );
        Ok(Some(service))
    }

    /// Look up an App by key, or fail with the keys that do exist.
    ///
    /// The message is read by an admin who has just named an App that is not
    /// there, so it lists what is.
    pub async fn require(&self, key: &str) -> Result<Arc<GitHubAppService>, AuthError> {
        match self.get(key).await? {
            Some(app) => Ok(app),
            None => Err(AuthError::BadRequest(format!(
                "Unknown GitHub App '{key}'. Registered: {:?}",
                self.keys().await.unwrap_or_default()
            ))),
        }
    }

    fn build(&self, record: &GitHubAppRecord) -> Result<GitHubAppService, AuthError> {
        let Some(cipher) = &self.cipher else {
            return Err(AuthError::Config(format!(
                "GitHub App '{}' is stored but this installation has no encryption configured, \
                 so its signing key cannot be read",
                record.app_key
            )));
        };
        let pem = record.private_key_pem(cipher.as_ref())?;
        GitHubAppService::from_record(record, &pem).ok_or_else(|| {
            AuthError::Config(format!(
                "The stored signing key for GitHub App '{}' could not be read",
                record.app_key
            ))
        })
    }

    /// The deployment-configured login App, if configured. This is what the
    /// sign-in path and user token refresh use.
    pub fn default_service(&self) -> Option<Arc<GitHubAppService>> {
        self.login_app.clone()
    }

    /// Every key currently available, login App included.
    pub async fn keys(&self) -> Result<Vec<String>, AuthError> {
        let mut keys: Vec<String> = self
            .login_app
            .as_ref()
            .map(|app| app.key().to_string())
            .into_iter()
            .collect();
        if let Some(store) = &self.store {
            keys.extend(store.list().await?.into_iter().map(|r| r.app_key));
        }
        keys.sort();
        keys.dedup();
        Ok(keys)
    }

    /// Every App currently available, in stable key order.
    ///
    /// An App whose stored key cannot be opened is skipped with a warning
    /// rather than failing the listing: one broken registration must not hide
    /// the rest, and the admin needs the list to fix it.
    pub async fn entries(&self) -> Result<Vec<(String, Arc<GitHubAppService>)>, AuthError> {
        let mut entries: Vec<(String, Arc<GitHubAppService>)> = Vec::new();
        if let Some(login_app) = &self.login_app {
            entries.push((login_app.key().to_string(), Arc::clone(login_app)));
        }
        if let Some(store) = &self.store {
            for record in store.list().await? {
                match self.get(&record.app_key).await {
                    Ok(Some(service)) => entries.push((record.app_key, service)),
                    Ok(None) => {}
                    Err(e) => {
                        warn!(
                            app_key = %record.app_key,
                            error = %e,
                            "Skipping GitHub App whose stored signing key could not be read"
                        );
                    }
                }
            }
        }
        entries.sort_by(|(left, _), (right, _)| left.cmp(right));
        Ok(entries)
    }

    /// Durable state behind admin-registered Apps, when this wiring has any.
    pub fn store(&self) -> Option<&Arc<dyn GitHubAppDao>> {
        self.store.as_ref()
    }

    /// The cipher stored keys are sealed with, when configured.
    pub fn cipher(&self) -> Option<&Arc<dyn SecretCipher>> {
        self.cipher.as_ref()
    }

    /// Forget an App's obtained tokens. Called after a registration changes,
    /// so the backend that took the request does not serve its own stale
    /// tokens until the next revision check.
    pub async fn forget(&self, key: &str) {
        self.cache.write().await.remove(key);
    }

    /// Move Apps configured through `GITHUB_APPS_JSON` into durable storage,
    /// once, on the first start after upgrading.
    ///
    /// A deployment that has been setting bot Apps through the environment
    /// keeps working without anyone editing anything; from then on the stored
    /// copy is what counts and the env var can be deleted. Removing the var
    /// afterwards does not remove the Apps, which is the point.
    ///
    /// This never fails a start-up. An App that cannot be carried is named in
    /// the log with the reason, and an admin registers it from the UI.
    pub async fn carry_over_env_apps(&self, entries: &[GitHubAppConfig]) {
        if entries.is_empty() {
            return;
        }
        let (Some(store), Some(cipher)) = (&self.store, &self.cipher) else {
            warn!(
                count = entries.len(),
                "GITHUB_APPS_JSON is set but this installation cannot store Apps, \
                 so they remain environment-only"
            );
            return;
        };

        // Everything that can fail per-entry happens before storage is touched:
        // an installation that cannot reach GitHub on this start has carried
        // nothing, and tries again on the next one rather than recording the
        // move as done.
        let mut records = Vec::new();
        for entry in entries {
            let Some(service) = GitHubAppService::from_app_config(entry) else {
                warn!(
                    app_key = %entry.key,
                    "Not carrying over GitHub App: its private key does not parse"
                );
                continue;
            };
            let app = match service.prove().await {
                Ok(app) => app,
                Err(e) => {
                    warn!(
                        app_key = %entry.key,
                        error = %e,
                        "Not carrying over GitHub App: GitHub did not confirm it"
                    );
                    continue;
                }
            };
            let new_app = NewGitHubApp {
                app_key: entry.key.clone(),
                app_id: app.app_id,
                name: app.name,
                slug: app.slug,
                bot_login: app.bot_login,
                private_key_pem: entry.private_key.clone(),
                default_installation_id: entry.default_installation_id,
            };
            match GitHubAppRecord::seal(new_app, cipher.as_ref(), None) {
                Ok(record) => records.push(record),
                Err(e) => {
                    warn!(app_key = %entry.key, error = %e, "Could not seal GitHub App for storage");
                }
            }
        }
        if records.is_empty() {
            return;
        }

        // The Apps and the marker land together. A failure here leaves the
        // marker unclaimed, so the next start carries them properly rather
        // than believing a migration that never happened.
        let detail = format!("{} App(s) proved", records.len());
        let skipped = match store
            .carry_over(ENV_CARRY_OVER, &detail, &records, ENV_CARRY_OVER)
            .await
        {
            Ok(Some(skipped)) => skipped,
            Ok(None) => return,
            Err(e) => {
                warn!(
                    error = %e,
                    "Could not carry GITHUB_APPS_JSON Apps into storage — will retry on the next start"
                );
                return;
            }
        };

        for app_key in &skipped {
            warn!(
                app_key = %app_key,
                "Not carrying over GitHub App: that key is already registered"
            );
        }
        info!(
            carried = records.len() - skipped.len(),
            configured = entries.len(),
            "Carried GITHUB_APPS_JSON Apps into storage — the environment variable can now be removed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::github::dao::test_support::InMemoryGitHubAppDao;
    use crate::auth::github::dao::{GitHubAppRecord, NewGitHubApp};
    use crate::config::SecretsConfig;
    use crate::secret::service::LocalEnvelopeSecretCipher;

    /// A parseable test key, so `from_record` produces a real service.
    const TEST_PEM: &str = include_str!("testdata/github_app_test_key.pem");

    fn cipher() -> Arc<dyn SecretCipher> {
        LocalEnvelopeSecretCipher::from_config(&SecretsConfig {
            encryption_key: Some("0123456789abcdef0123456789abcdef".into()),
            kms_key_id: None,
        })
        .expect("cipher")
    }

    fn record(app_key: &str, cipher: &dyn SecretCipher) -> GitHubAppRecord {
        GitHubAppRecord::seal(
            NewGitHubApp {
                app_key: app_key.to_string(),
                app_id: 42,
                name: "Reviewer".into(),
                slug: "reviewer".into(),
                bot_login: "reviewer[bot]".into(),
                private_key_pem: TEST_PEM.to_string(),
                default_installation_id: None,
            },
            cipher,
            None,
        )
        .expect("seal")
    }

    fn registry(store: Arc<InMemoryGitHubAppDao>, cipher: Arc<dyn SecretCipher>) -> GitHubAppRegistry {
        GitHubAppRegistry {
            login_app: None,
            store: Some(store),
            cipher: Some(cipher),
            cache: RwLock::new(HashMap::new()),
        }
    }

    #[tokio::test]
    async fn an_empty_registry_finds_nothing_and_says_so_plainly() {
        let registry = GitHubAppRegistry::default();
        assert!(registry.get("reviewer").await.expect("get").is_none());
        assert!(registry.default_service().is_none());
        assert!(registry.keys().await.expect("keys").is_empty());
    }

    #[tokio::test]
    async fn an_app_registered_after_the_registry_was_built_is_found() {
        // The whole point of decision 2: no restart, no reload call.
        let store = Arc::new(InMemoryGitHubAppDao::default());
        let cipher = cipher();
        let registry = registry(Arc::clone(&store), Arc::clone(&cipher));

        assert!(registry.get("reviewer").await.expect("get").is_none());
        store.put(record("reviewer", cipher.as_ref()));
        assert!(registry.get("reviewer").await.expect("get").is_some());
    }

    #[tokio::test]
    async fn an_unchanged_record_keeps_the_same_live_service() {
        // Obtained tokens live inside the service, so an unchanged record must
        // return the same instance rather than a fresh one.
        let store = Arc::new(InMemoryGitHubAppDao::default());
        let cipher = cipher();
        let registry = registry(Arc::clone(&store), Arc::clone(&cipher));
        store.put(record("reviewer", cipher.as_ref()));

        let first = registry.get("reviewer").await.expect("get").expect("app");
        let second = registry.get("reviewer").await.expect("get").expect("app");
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn an_edited_record_replaces_only_its_own_service() {
        let store = Arc::new(InMemoryGitHubAppDao::default());
        let cipher = cipher();
        let registry = registry(Arc::clone(&store), Arc::clone(&cipher));
        store.put(record("reviewer", cipher.as_ref()));
        store.put(record("releaser", cipher.as_ref()));

        let reviewer_before = registry.get("reviewer").await.expect("get").expect("app");
        let releaser_before = registry.get("releaser").await.expect("get").expect("app");

        store.bump_revision("reviewer");

        let reviewer_after = registry.get("reviewer").await.expect("get").expect("app");
        let releaser_after = registry.get("releaser").await.expect("get").expect("app");

        assert!(!Arc::ptr_eq(&reviewer_before, &reviewer_after));
        assert!(Arc::ptr_eq(&releaser_before, &releaser_after));
    }

    #[tokio::test]
    async fn a_key_reused_by_a_new_registration_does_not_serve_the_deleted_one() {
        // Removing an App and registering another under the same name produces
        // a different record that also starts at revision 1. Keying the cache
        // on the revision alone would go on minting tokens with the deleted
        // signing key.
        let store = Arc::new(InMemoryGitHubAppDao::default());
        let cipher = cipher();
        let registry = registry(Arc::clone(&store), Arc::clone(&cipher));
        store.put(record("reviewer", cipher.as_ref()));

        let before = registry.get("reviewer").await.expect("get").expect("app");

        let replacement = record("reviewer", cipher.as_ref());
        assert_eq!(replacement.revision, 1, "a new record starts at revision 1");
        store.delete("reviewer").await.expect("delete");
        store.put(replacement);

        let after = registry.get("reviewer").await.expect("get").expect("app");
        assert!(!Arc::ptr_eq(&before, &after));
    }

    #[tokio::test]
    async fn a_failed_carry_over_leaves_nothing_claimed_so_the_next_start_retries() {
        // Recording the move as done before the Apps are written would lose
        // them permanently: the environment variable is still set, and nothing
        // would ever read it again.
        let store = Arc::new(InMemoryGitHubAppDao::default());
        let registry = registry(Arc::clone(&store), cipher());
        *store.fail_writes.lock().expect("lock") = true;

        let entries = vec![GitHubAppConfig {
            key: "env-bot".into(),
            app_id: 42,
            private_key: TEST_PEM.to_string(),
            client_id: None,
            client_secret: None,
            default_installation_id: None,
        }];
        // `prove` cannot reach GitHub here, so this stops before storage — which
        // is itself the guarantee: an unreachable GitHub claims nothing either.
        registry.carry_over_env_apps(&entries).await;

        assert!(!store.migration_claimed(ENV_CARRY_OVER));
        assert!(store.list().await.expect("list").is_empty());
    }

    #[tokio::test]
    async fn a_removed_app_stops_answering_even_though_it_was_cached() {
        let store = Arc::new(InMemoryGitHubAppDao::default());
        let cipher = cipher();
        let registry = registry(Arc::clone(&store), Arc::clone(&cipher));
        store.put(record("reviewer", cipher.as_ref()));

        assert!(registry.get("reviewer").await.expect("get").is_some());
        store.delete("reviewer").await.expect("delete");
        assert!(registry.get("reviewer").await.expect("get").is_none());
    }

    #[tokio::test]
    async fn an_unreadable_store_is_an_error_not_an_absent_app() {
        // Collapsing these two would tell an admin to register an App that is
        // already there, every time the database hiccups.
        let store = Arc::new(InMemoryGitHubAppDao::default());
        let cipher = cipher();
        let registry = registry(Arc::clone(&store), Arc::clone(&cipher));
        store.put(record("reviewer", cipher.as_ref()));
        *store.fail_reads.lock().expect("lock") = true;

        assert!(registry.get("reviewer").await.is_err());
    }

    #[tokio::test]
    async fn a_stored_app_without_encryption_configured_fails_loudly() {
        let store = Arc::new(InMemoryGitHubAppDao::default());
        let cipher = cipher();
        store.put(record("reviewer", cipher.as_ref()));
        let registry = GitHubAppRegistry {
            login_app: None,
            store: Some(store),
            cipher: None,
            cache: RwLock::new(HashMap::new()),
        };

        let Err(err) = registry.get("reviewer").await else {
            panic!("a stored App with no cipher must not resolve");
        };
        assert!(err.to_string().contains("encryption"));
    }

    #[tokio::test]
    async fn require_names_the_apps_that_do_exist() {
        let store = Arc::new(InMemoryGitHubAppDao::default());
        let cipher = cipher();
        let registry = registry(Arc::clone(&store), Arc::clone(&cipher));
        store.put(record("reviewer", cipher.as_ref()));

        let Err(err) = registry.require("releaser").await else {
            panic!("an unregistered key must not resolve");
        };
        assert!(err.to_string().contains("reviewer"));
    }
}
