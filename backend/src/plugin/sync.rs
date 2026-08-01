//! The one routine that turns a plugin's git content into stored skills.
//!
//! Every reader of a plugin's skills goes through [`PluginSyncService::skills`]
//! and nothing else, so there is a single answer to "what is in this plugin"
//! and a single place where fetching is decided.
//!
//! The sequence is: ask the source what version it is at, compare against what
//! was stored, and fetch only on a difference. Fetching uses the platform's own
//! credential rather than the caller's, because a plugin an administrator
//! turned on for everyone may live in a repository the caller cannot read.
//!
//! Concurrent callers for the same plugin are serialised on a per-plugin lock.
//! The second caller re-reads the stored version after the first one finishes,
//! finds it current, and fetches nothing — so a burst of callers arriving at a
//! changed version costs exactly one fetch.

use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use super::dao::PluginDao;
use super::models::{Plugin, PluginSkill};
use crate::auth::credentials::resolve_repository_github_scope_token;
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::ProviderTokenAuthDao;
use crate::credentials::github::scope::GitHubScope;
use crate::repository::access::parse_github_owner_repo;
use crate::secret::service::SecretService;
use crate::skill::error::SkillError;
use crate::skill::fetcher::{SkillFetcher, SkillLayout};
use crate::skill::models::SkillSource;
use crate::skill::scanner;

/// Syncs plugin content into stored skills, and reads them back.
pub struct PluginSyncService {
    plugins: Arc<dyn PluginDao>,
    fetcher: Arc<dyn SkillFetcher>,
    provider_tokens: Arc<dyn ProviderTokenDao>,
    provider_token_auth: Arc<dyn ProviderTokenAuthDao>,
    github_app_service: Option<Arc<GitHubAppService>>,
    github_apps: Arc<GitHubAppRegistry>,
    secrets: Arc<SecretService>,
    /// One lock per plugin id, so simultaneous callers collapse into one fetch.
    /// Held in a bounded cache rather than a map that only ever grows; evicting
    /// an idle lock is harmless because nobody is waiting on it.
    locks: Cache<String, Arc<Mutex<()>>>,
}

impl PluginSyncService {
    #[allow(clippy::too_many_arguments)]
    // reason: mirrors the credential-resolution dependencies every GitHub
    // caller in this codebase takes; constructed once from AppState::new.
    pub fn new(
        plugins: Arc<dyn PluginDao>,
        fetcher: Arc<dyn SkillFetcher>,
        provider_tokens: Arc<dyn ProviderTokenDao>,
        provider_token_auth: Arc<dyn ProviderTokenAuthDao>,
        github_app_service: Option<Arc<GitHubAppService>>,
        github_apps: Arc<GitHubAppRegistry>,
        secrets: Arc<SecretService>,
    ) -> Self {
        Self {
            plugins,
            fetcher,
            provider_tokens,
            provider_token_auth,
            github_app_service,
            github_apps,
            secrets,
            locks: Cache::builder()
                .time_to_idle(Duration::from_mins(5))
                .max_capacity(1024)
                .build(),
        }
    }

    /// The skills in a plugin, syncing first if the source has moved on.
    ///
    /// A failed sync is not an error: the previous snapshot is served and the
    /// failure logged, so one unreachable plugin does not empty a skill list
    /// that other plugins contributed to.
    pub async fn skills(&self, plugin: &Plugin) -> Result<Vec<PluginSkill>, sqlx::Error> {
        if let Err(e) = self.sync(plugin).await {
            warn!(
                plugin_id = %plugin.id,
                git_url = %plugin.git_url,
                error = %e,
                "Plugin sync failed — serving the last stored snapshot"
            );
        }
        self.plugins.list_skills(&plugin.id).await
    }

    /// Bring a plugin's stored skills up to date with its source.
    ///
    /// Returns without fetching when the source reports the version already
    /// stored. Errors are the caller's to decide about; [`Self::skills`]
    /// downgrades them to a warning.
    pub async fn sync(&self, plugin: &Plugin) -> Result<(), SkillError> {
        let lock = self
            .locks
            .get_with(plugin.id.clone(), async { Arc::new(Mutex::new(())) })
            .await;
        let _guard = lock.lock().await;

        // Re-read under the lock: a caller that queued behind a sync for the
        // same plugin is looking at a version from before that sync ran.
        let current = self
            .plugins
            .find_by_id(&plugin.id)
            .await
            .map_err(SkillError::Db)?
            .unwrap_or_else(|| plugin.clone());

        let token = self.platform_token(&current.git_url).await;

        let version = self
            .fetcher
            .fetch_version(&current.git_url, token.as_deref())
            .await?;

        if let (Some(fetched), Some(stored)) =
            (version.as_deref(), current.synced_version.as_deref())
        {
            if fetched == stored {
                debug!(
                    plugin_id = %current.id,
                    version = fetched,
                    "Plugin unchanged — skipping fetch"
                );
                return Ok(());
            }
        }

        let files = self
            .fetcher
            .fetch_skill_files(&current.git_url, SkillLayout::Plugin, token.as_deref())
            .await?;

        // A stored skill is keyed on its directory name, not the `name:` in its
        // frontmatter. That is the identity everything downstream already uses
        // — the sandbox removes directories, Codex staging names them — and it
        // is the one upstream cannot change without the activation records
        // pointing at it plainly going stale.
        let mut skills: Vec<PluginSkill> = files
            .into_iter()
            .filter_map(|f| {
                scanner::parse_skill(&f.content, &f.dir_name, SkillSource::Plugin, None, None).map(
                    |s| PluginSkill {
                        plugin_id: current.id.clone(),
                        name: f.dir_name,
                        description: s.description,
                    },
                )
            })
            .collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        skills.dedup_by(|a, b| a.name == b.name);

        self.plugins
            .replace_skills(&current.id, version.clone(), &skills)
            .await
            .map_err(SkillError::Db)?;

        debug!(
            plugin_id = %current.id,
            skill_count = skills.len(),
            "Plugin synced"
        );
        Ok(())
    }

    /// The platform's own GitHub credential for a plugin's repository.
    ///
    /// `None` when no global credential is configured or it cannot be minted —
    /// a public plugin still fetches unauthenticated, and a private one fails
    /// at the fetch with a message about authentication rather than here.
    async fn platform_token(&self, git_url: &str) -> Option<String> {
        let (owner, repo) = parse_github_owner_repo(git_url)?;
        match resolve_repository_github_scope_token(
            self.provider_tokens.as_ref(),
            Some(self.provider_token_auth.as_ref()),
            self.github_app_service.as_deref(),
            self.github_apps.as_ref(),
            Some(self.secrets.as_ref()),
            &GitHubScope::Global,
            &owner,
            &repo,
        )
        .await
        {
            Ok(token) => token,
            Err(e) => {
                debug!(
                    git_url,
                    error = %e,
                    "No platform GitHub credential for plugin — fetching unauthenticated"
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;

    use super::super::models::ActivationRow;
    use super::*;
    use crate::auth::provider_tokens::MockProviderTokenAuthDao;
    use crate::auth::test_support::StubProviderTokens;
    use crate::secret::test_support::service_with;
    use crate::skill::fetcher::FetchedSkillFile;

    const SKILL_MD: &str =
        "---\nname: sample-skill\ndescription: Runs the sample workflow\n---\nbody";

    /// In-memory plugin store that behaves like the real one for the two things
    /// these tests turn on: `find_by_id` reflects the last `replace_skills`,
    /// and `replace_skills` is atomic from a caller's point of view.
    #[derive(Default)]
    struct StubPlugins {
        plugin: StdMutex<Option<Plugin>>,
        skills: StdMutex<Vec<PluginSkill>>,
    }

    impl StubPlugins {
        fn with(plugin: Plugin) -> Self {
            Self {
                plugin: StdMutex::new(Some(plugin)),
                skills: StdMutex::new(Vec::new()),
            }
        }

        fn seed_skills(self, skills: Vec<PluginSkill>) -> Self {
            *self.skills.lock().unwrap() = skills;
            self
        }
    }

    #[async_trait]
    impl PluginDao for StubPlugins {
        async fn list_all(&self) -> Result<Vec<Plugin>, sqlx::Error> {
            unimplemented!()
        }

        async fn find_by_id(&self, _id: &str) -> Result<Option<Plugin>, sqlx::Error> {
            Ok(self.plugin.lock().unwrap().clone())
        }

        async fn insert(&self, _row: &Plugin) -> Result<(), sqlx::Error> {
            unimplemented!()
        }

        async fn delete(&self, _id: &str) -> Result<bool, sqlx::Error> {
            unimplemented!()
        }

        async fn list_skills(&self, _plugin_id: &str) -> Result<Vec<PluginSkill>, sqlx::Error> {
            Ok(self.skills.lock().unwrap().clone())
        }

        async fn replace_skills(
            &self,
            _plugin_id: &str,
            version: Option<String>,
            skills: &[PluginSkill],
        ) -> Result<(), sqlx::Error> {
            *self.skills.lock().unwrap() = skills.to_vec();
            if let Some(p) = self.plugin.lock().unwrap().as_mut() {
                p.synced_version = version;
            }
            Ok(())
        }

        // Activation is resolved elsewhere; these tests are about fetching.
        async fn list_all_skills(&self) -> Result<Vec<PluginSkill>, sqlx::Error> {
            unimplemented!()
        }

        async fn list_activations_for(
            &self,
            _user_id: Option<uuid::Uuid>,
        ) -> Result<Vec<ActivationRow>, sqlx::Error> {
            unimplemented!()
        }

        async fn set_activation(&self, _row: &ActivationRow) -> Result<(), sqlx::Error> {
            unimplemented!()
        }

        async fn clear_activation(
            &self,
            _plugin_id: &str,
            _skill_name: Option<String>,
            _user_id: Option<uuid::Uuid>,
        ) -> Result<bool, sqlx::Error> {
            unimplemented!()
        }
    }

    /// Counts what it was asked to do, so a test can assert on fetches rather
    /// than on results that several code paths could produce.
    struct CountingFetcher {
        version: StdMutex<String>,
        versions_probed: AtomicUsize,
        files_fetched: AtomicUsize,
        fail_fetch: bool,
    }

    impl CountingFetcher {
        fn at_version(version: &str) -> Self {
            Self {
                version: StdMutex::new(version.to_string()),
                versions_probed: AtomicUsize::new(0),
                files_fetched: AtomicUsize::new(0),
                fail_fetch: false,
            }
        }

        fn failing() -> Self {
            Self {
                fail_fetch: true,
                ..Self::at_version("v2")
            }
        }
    }

    #[async_trait]
    impl SkillFetcher for CountingFetcher {
        async fn fetch_skill_files(
            &self,
            remote_url: &str,
            _layout: SkillLayout,
            _token: Option<&str>,
        ) -> Result<Vec<FetchedSkillFile>, SkillError> {
            self.files_fetched.fetch_add(1, Ordering::SeqCst);
            if self.fail_fetch {
                return Err(SkillError::Fetch {
                    git_url: remote_url.to_string(),
                    message: "boom".to_string(),
                });
            }
            // Slow enough that a second caller reliably queues behind the
            // first rather than racing it.
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(vec![FetchedSkillFile {
                dir_name: "sample-skill".to_string(),
                content: SKILL_MD.to_string(),
            }])
        }

        async fn fetch_version(
            &self,
            _remote_url: &str,
            _token: Option<&str>,
        ) -> Result<Option<String>, SkillError> {
            self.versions_probed.fetch_add(1, Ordering::SeqCst);
            Ok(Some(self.version.lock().unwrap().clone()))
        }
    }

    fn plugin(synced_version: Option<&str>) -> Plugin {
        Plugin {
            id: "p1".to_string(),
            name: "moatless-skills".to_string(),
            git_url: "https://github.com/acme/plugins".to_string(),
            synced_version: synced_version.map(str::to_string),
            synced_at: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn service(
        plugins: Arc<dyn PluginDao>,
        fetcher: Arc<dyn SkillFetcher>,
    ) -> PluginSyncService {
        PluginSyncService::new(
            plugins,
            fetcher,
            // No credential rows: the platform token resolves to None and the
            // fetch goes out unauthenticated, which is not what these tests
            // are about.
            Arc::new(StubProviderTokens::with_user_row(None)),
            Arc::new(MockProviderTokenAuthDao::new()),
            None,
            Arc::new(GitHubAppRegistry::default()),
            Arc::new(service_with(vec![])),
        )
    }

    #[tokio::test]
    async fn a_second_call_at_an_unchanged_version_performs_no_fetch() {
        let plugins = Arc::new(StubPlugins::with(plugin(None)));
        let fetcher = Arc::new(CountingFetcher::at_version("sha-1"));
        let sync = service(plugins.clone(), fetcher.clone());

        sync.sync(&plugin(None)).await.unwrap();
        sync.sync(&plugin(None)).await.unwrap();

        assert_eq!(
            fetcher.files_fetched.load(Ordering::SeqCst),
            1,
            "the first call had nothing stored so it fetched; the second saw \
             the same version and must not have"
        );
        assert_eq!(
            fetcher.versions_probed.load(Ordering::SeqCst),
            2,
            "both calls still ask the source what version it is at — that is \
             the cheap call the design spends"
        );
    }

    #[tokio::test]
    async fn a_changed_version_is_fetched_and_stored() {
        let plugins = Arc::new(StubPlugins::with(plugin(Some("sha-old"))));
        let fetcher = Arc::new(CountingFetcher::at_version("sha-new"));
        let sync = service(plugins.clone(), fetcher.clone());

        sync.sync(&plugin(Some("sha-old"))).await.unwrap();

        assert_eq!(fetcher.files_fetched.load(Ordering::SeqCst), 1);
        let stored = plugins.list_skills("p1").await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].name, "sample-skill");
        assert_eq!(stored[0].description, "Runs the sample workflow");
        assert_eq!(
            plugins.find_by_id("p1").await.unwrap().unwrap().synced_version,
            Some("sha-new".to_string()),
            "the version is recorded with the skills it produced"
        );
    }

    #[tokio::test]
    async fn two_simultaneous_callers_at_a_changed_version_fetch_once() {
        let plugins = Arc::new(StubPlugins::with(plugin(Some("sha-old"))));
        let fetcher = Arc::new(CountingFetcher::at_version("sha-new"));
        let sync = Arc::new(service(plugins.clone(), fetcher.clone()));

        let (a, b) = tokio::join!(
            {
                let sync = sync.clone();
                async move { sync.sync(&plugin(Some("sha-old"))).await }
            },
            {
                let sync = sync.clone();
                async move { sync.sync(&plugin(Some("sha-old"))).await }
            }
        );
        a.unwrap();
        b.unwrap();

        assert_eq!(
            fetcher.files_fetched.load(Ordering::SeqCst),
            1,
            "the second caller waits on the per-plugin lock, then re-reads the \
             version the first one just stored and finds nothing to do"
        );
    }

    #[tokio::test]
    async fn a_failed_fetch_leaves_the_previous_snapshot_in_place() {
        let previous = vec![PluginSkill {
            plugin_id: "p1".to_string(),
            name: "code-review".to_string(),
            description: "Reviews code".to_string(),
        }];
        let plugins =
            Arc::new(StubPlugins::with(plugin(Some("sha-old"))).seed_skills(previous.clone()));
        let sync = service(plugins.clone(), Arc::new(CountingFetcher::failing()));

        let skills = sync.skills(&plugin(Some("sha-old"))).await.unwrap();

        assert_eq!(
            skills, previous,
            "an unreachable plugin must not empty a skill list other plugins \
             also contribute to"
        );
    }
}
