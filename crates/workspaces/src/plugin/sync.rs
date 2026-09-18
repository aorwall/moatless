//! The one routine that turns a plugin's git content into stored skills.
//!
//! Every reader goes through [`PluginSyncService::skills`], the single place
//! fetching is decided: compare the source's version against what's stored and
//! fetch only on a difference, running as the caller so an unreadable repo
//! fails naming authentication.

use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use super::dao::PluginDao;
use super::models::{Plugin, PluginSkill};
use crate::skill::error::SkillError;
use crate::skill::fetcher::{SkillFetcher, SkillLayout};
use crate::skill::models::SkillSource;
use crate::skill::scanner;
use moatless_ports::identity::GitHostTokenIssuer;
use moatless_types::access_control::Actor;
use moatless_workspace_types::repository::RepositoryProvider;
use moatless_workspace_types::repository::RepositoryRemote;

/// Syncs plugin content into stored skills, and reads them back.
pub struct PluginSyncService {
    plugins: Arc<dyn PluginDao>,
    fetcher: Arc<dyn SkillFetcher>,
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    /// One lock per plugin id, so simultaneous callers collapse into one fetch.
    /// Held in a bounded cache rather than a map that only ever grows; evicting
    /// an idle lock is harmless because nobody is waiting on it.
    locks: Cache<String, Arc<Mutex<()>>>,
}

impl PluginSyncService {
    pub(crate) fn new(
        plugins: Arc<dyn PluginDao>,
        fetcher: Arc<dyn SkillFetcher>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    ) -> Self {
        Self {
            plugins,
            fetcher,
            git_host_tokens,
            locks: Cache::builder()
                .time_to_idle(Duration::from_mins(5))
                .max_capacity(1024)
                .build(),
        }
    }

    /// The skills in a plugin, syncing first if the source has moved on.
    ///
    /// A failed sync is not an error: the previous snapshot is served and
    /// logged, so one unreachable plugin does not empty a skill list others
    /// contributed to.
    pub async fn skills(
        &self,
        actor: &Actor,
        plugin: &Plugin,
    ) -> Result<Vec<PluginSkill>, sqlx::Error> {
        if let Err(e) = self.sync(actor, plugin).await {
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
    pub async fn sync(&self, actor: &Actor, plugin: &Plugin) -> Result<(), SkillError> {
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

        let remote = crate::repository::remote::resolve_remote(
            Some(RepositoryProvider::Github),
            current.git_url.clone(),
        );
        let token = self.fetch_token(actor, &remote).await;

        let version = self
            .fetcher
            .fetch_version(&remote, token.as_deref())
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
            .fetch_skill_files(&remote, SkillLayout::Plugin, token.as_deref())
            .await?;

        // A stored skill is keyed on its directory name, not the `name:` in its
        // frontmatter. That is the identity everything downstream already uses
        // — the sandbox removes directories, Codex staging names them — and it
        // is the one upstream cannot change without the activation records
        // pointing at it plainly going stale.
        let mut skills: Vec<PluginSkill> = files
            .into_iter()
            .filter_map(|f| {
                scanner::parse_skill(
                    &f.content,
                    &f.dir_name,
                    &f.path,
                    SkillSource::Plugin,
                    None,
                    None,
                )
                .map(|s| PluginSkill {
                    plugin_id: current.id.clone(),
                    name: f.dir_name,
                    description: s.description,
                    path: s.path,
                })
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

    /// `actor`'s own credential for a plugin's repository.
    ///
    /// `None` when unconnected or unmintable — a public plugin still fetches
    /// unauthenticated; a private one fails at the fetch naming authentication,
    /// not here.
    async fn fetch_token(&self, actor: &Actor, remote: &RepositoryRemote) -> Option<String> {
        let repo = remote.host_repo.as_ref()?;
        match self
            .git_host_tokens
            .token(actor, &repo.credential_account())
            .await
        {
            Ok(token) => Some(token),
            Err(e) => {
                debug!(
                    git_url = %remote.url,
                    %actor,
                    error = %e,
                    "No GitHub credential for plugin — fetching unauthenticated"
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
    use crate::skill::fetcher::FetchedSkillFile;
    use moatless_identity::auth::test_support::StubProviderTokens;

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
            _workspace_id: Option<&str>,
        ) -> Result<Vec<ActivationRow>, sqlx::Error> {
            unimplemented!()
        }

        async fn set_activation(
            &self,
            _write: &crate::plugin::models::ActivationWrite,
        ) -> Result<ActivationRow, sqlx::Error> {
            unimplemented!()
        }

        async fn clear_activation(
            &self,
            _plugin_id: &str,
            _skill_name: Option<String>,
            _user_id: Option<uuid::Uuid>,
            _workspace_id: Option<&str>,
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
            remote: &RepositoryRemote,
            _layout: SkillLayout,
            _token: Option<&str>,
        ) -> Result<Vec<FetchedSkillFile>, SkillError> {
            self.files_fetched.fetch_add(1, Ordering::SeqCst);
            if self.fail_fetch {
                return Err(SkillError::Fetch {
                    git_url: remote.url.clone(),
                    message: "boom".to_string(),
                });
            }
            // Slow enough that a second caller reliably queues behind the
            // first rather than racing it.
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(vec![FetchedSkillFile {
                dir_name: "sample-skill".to_string(),
                path: "skills/sample-skill/SKILL.md".to_string(),
                content: SKILL_MD.to_string(),
            }])
        }

        async fn fetch_version(
            &self,
            _remote: &RepositoryRemote,
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

    fn service(plugins: Arc<dyn PluginDao>, fetcher: Arc<dyn SkillFetcher>) -> PluginSyncService {
        PluginSyncService::new(
            plugins,
            fetcher,
            // No credential rows: the token resolves to None and the fetch
            // goes out unauthenticated, which is not what these tests are
            // about.
            moatless_identity::git_host_tokens::seam_over(Arc::new(
                StubProviderTokens::with_user_row(None),
            )),
        )
    }

    fn actor() -> Actor {
        Actor::of("u1")
    }

    #[tokio::test]
    async fn a_second_call_at_an_unchanged_version_performs_no_fetch() {
        let plugins = Arc::new(StubPlugins::with(plugin(None)));
        let fetcher = Arc::new(CountingFetcher::at_version("sha-1"));
        let sync = service(plugins.clone(), fetcher.clone());

        sync.sync(&actor(), &plugin(None)).await.unwrap();
        sync.sync(&actor(), &plugin(None)).await.unwrap();

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

        sync.sync(&actor(), &plugin(Some("sha-old"))).await.unwrap();

        assert_eq!(fetcher.files_fetched.load(Ordering::SeqCst), 1);
        let stored = plugins.list_skills("p1").await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].name, "sample-skill");
        assert_eq!(stored[0].description, "Runs the sample workflow");
        assert_eq!(
            plugins
                .find_by_id("p1")
                .await
                .unwrap()
                .unwrap()
                .synced_version,
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
                async move { sync.sync(&actor(), &plugin(Some("sha-old"))).await }
            },
            {
                let sync = sync.clone();
                async move { sync.sync(&actor(), &plugin(Some("sha-old"))).await }
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
            path: "skills/code-review/SKILL.md".to_string(),
        }];
        let plugins =
            Arc::new(StubPlugins::with(plugin(Some("sha-old"))).seed_skills(previous.clone()));
        let sync = service(plugins.clone(), Arc::new(CountingFetcher::failing()));

        let skills = sync
            .skills(&actor(), &plugin(Some("sha-old")))
            .await
            .unwrap();

        assert_eq!(
            skills, previous,
            "an unreachable plugin must not empty a skill list other plugins \
             also contribute to"
        );
    }
}
