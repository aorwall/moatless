//! Registry of configured GitHub Apps, keyed by stable string.
//!
//! The default user-auth App (configured via the existing `GITHUB_APP_*` env
//! vars) is registered under [`GitHubAppService::DEFAULT_KEY`] (`"default"`).
//! Additional "bot Apps" — separate App registrations whose bot identities
//! triggers can run as — are loaded from `GITHUB_APPS_JSON` (a JSON array of
//! [`GitHubAppConfig`]).
//!
//! All existing single-App call sites (user OAuth refresh, etc.) go through
//! [`GitHubAppRegistry::default_service`]; bot-mode call sites pick an App by
//! key with [`GitHubAppRegistry::get`].

use std::collections::HashMap;
use std::sync::Arc;

use tracing::warn;

use crate::auth::github_app_service::GitHubAppService;
use crate::config::Config;

/// Registry of configured GitHub Apps.
#[derive(Default)]
pub struct GitHubAppRegistry {
    apps: HashMap<String, Arc<GitHubAppService>>,
    default_key: Option<String>,
}

impl GitHubAppRegistry {
    /// Build the registry from `Config`.
    ///
    /// Loads:
    /// 1. The legacy single-App config (`GITHUB_APP_*` env vars) under
    ///    [`GitHubAppService::DEFAULT_KEY`], if `github_app_id` is set.
    /// 2. Each entry in `config.auth.github_apps` (parsed from
    ///    `GITHUB_APPS_JSON`). Entries with the default key are skipped if
    ///    the legacy config already populated it.
    pub fn from_config(config: &Config) -> Self {
        let mut apps: HashMap<String, Arc<GitHubAppService>> = HashMap::new();
        let mut default_key: Option<String> = None;

        if let Some(svc) = GitHubAppService::from_config(config).map(Arc::new) {
            let key = svc.key().to_string();
            default_key = Some(key.clone());
            apps.insert(key, svc);
        }

        for entry in &config.auth.github_apps {
            if apps.contains_key(&entry.key) {
                warn!(
                    app_key = %entry.key,
                    "Duplicate GitHub App key — skipping (already loaded from legacy config)"
                );
                continue;
            }
            match GitHubAppService::from_app_config(entry).map(Arc::new) {
                Some(svc) => {
                    apps.insert(entry.key.clone(), svc);
                }
                None => {
                    warn!(app_key = %entry.key, "Skipping GitHub App entry with invalid PEM");
                }
            }
        }

        Self { apps, default_key }
    }

    /// Look up an App by its registry key. The key is the stable identifier
    /// from `GitHubAppConfig.key` (e.g. `"dev-bot"`); it surfaces in bot users'
    /// `provider_tokens.provider_user_id` so credential resolution can pick
    /// the right App.
    pub fn get(&self, key: &str) -> Option<Arc<GitHubAppService>> {
        self.apps.get(key).cloned()
    }

    /// The default user-auth App, if configured. This is what existing
    /// single-App call sites (user OAuth token refresh) use.
    pub fn default_service(&self) -> Option<Arc<GitHubAppService>> {
        self.default_key.as_deref().and_then(|k| self.get(k))
    }

    /// Whether any App is registered under `key`.
    pub fn contains(&self, key: &str) -> bool {
        self.apps.contains_key(key)
    }

    /// All registered keys (used by validation error messages).
    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.apps.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Whether the registry is empty (no Apps configured at all).
    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_reports_empty() {
        let registry = GitHubAppRegistry::default();
        assert!(registry.is_empty());
        assert!(registry.default_service().is_none());
        assert!(registry.get("dev-bot").is_none());
        assert!(registry.keys().is_empty());
    }
}
