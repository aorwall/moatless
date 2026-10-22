//! The one routine that turns a plugin's git content into stored skills.
//!
//! Every reader goes through [`PluginSyncService::skills`], the single place
//! fetching is decided: compare the source's version against what's stored and
//! fetch only on a difference, running as the caller so an unreadable repo
//! fails naming authentication. The comparison runs at most once per
//! [`CHECK_INTERVAL`] per plugin and gives up after [`CHECK_TIMEOUT`].

use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;
use tokio::sync::Mutex;
use tokio::time::Instant;
use tracing::{debug, warn};

use super::dao::PluginDao;
use super::models::{Plugin, PluginSkill};
use crate::skill::error::SkillError;
use crate::skill::fetcher::{SkillFetcher, SkillLayout};
use crate::skill::models::SkillSource;
use crate::skill::scanner;
use moatless_ports::identity::{GitHostTokenError, GitHostTokenIssuer};
use moatless_types::access_control::Actor;
use moatless_workspace_types::repository::RepositoryProvider;
use moatless_workspace_types::repository::RepositoryRemote;

/// How long a plugin's stored skills are served before its source is asked
/// again whether it moved on.
const CHECK_INTERVAL: Duration = Duration::from_mins(1);

/// The most an update check (token mint plus HEAD lookup) may add to a read
/// before the stored snapshot is served instead.
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// Syncs plugin content into stored skills, and reads them back.
pub struct PluginSyncService {
    plugins: Arc<dyn PluginDao>,
    fetcher: Arc<dyn SkillFetcher>,
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    /// One lock per plugin id, so simultaneous callers collapse into one fetch.
    /// Held in a bounded cache rather than a map that only ever grows; evicting
    /// an idle lock is harmless because nobody is waiting on it.
    locks: Cache<String, Arc<Mutex<()>>>,
    /// When each plugin's source was last asked for its version, whatever the
    /// answer: a source that failed or timed out is not asked again until
    /// `check_interval` has passed either.
    checked: Cache<String, Instant>,
    check_interval: Duration,
    check_timeout: Duration,
}

impl PluginSyncService {
    pub(crate) fn new(
        plugins: Arc<dyn PluginDao>,
        fetcher: Arc<dyn SkillFetcher>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    ) -> Self {
        Self::with_timing(
            plugins,
            fetcher,
            git_host_tokens,
            CHECK_INTERVAL,
            CHECK_TIMEOUT,
        )
    }

    fn with_timing(
        plugins: Arc<dyn PluginDao>,
        fetcher: Arc<dyn SkillFetcher>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
        check_interval: Duration,
        check_timeout: Duration,
    ) -> Self {
        Self {
            plugins,
            fetcher,
            git_host_tokens,
            locks: Cache::builder()
                .time_to_idle(Duration::from_mins(5))
                .max_capacity(1024)
                .build(),
            checked: Cache::builder().max_capacity(1024).build(),
            check_interval,
            check_timeout,
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
    /// Returns without fetching when the source was checked within
    /// `check_interval` or reports the version already stored. Errors are the
    /// caller's to decide about; [`Self::skills`] downgrades them to a warning.
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

        let recently_checked = self
            .checked
            .get(&current.id)
            .await
            .is_some_and(|at| at.elapsed() < self.check_interval);
        if recently_checked {
            return Ok(());
        }
        // Marked before the check so a failed one backs off too (see
        // `checked`). A plugin with nothing stored has no snapshot to fall
        // back on, so it is only marked once a sync succeeds.
        if current.synced_version.is_some() {
            self.checked
                .insert(current.id.clone(), Instant::now())
                .await;
        }

        let remote = plugin_remote(&current.git_url);
        let check = async {
            let token = plugin_token(self.git_host_tokens.as_ref(), actor, &remote).await?;
            let version = self
                .fetcher
                .fetch_version(&remote, token.as_deref())
                .await?;
            Ok::<_, SkillError>((token, version))
        };
        let checked = tokio::time::timeout(self.check_timeout, check)
            .await
            .unwrap_or_else(|_| {
                Err(SkillError::Fetch {
                    git_url: current.git_url.clone(),
                    message: format!("update check timed out after {:?}", self.check_timeout),
                })
            });
        let (token, version) = checked?;

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

        // Refused rather than stored: an empty list drops the plugin from every
        // sandbox config, and the recorded version stops later syncs refetching.
        if skills.is_empty() {
            let stored = self
                .plugins
                .list_skills(&current.id)
                .await
                .map_err(SkillError::Db)?;
            if !stored.is_empty() {
                return Err(SkillError::Fetch {
                    git_url: current.git_url.clone(),
                    message: format!(
                        "found no skills; keeping the {} stored and not recording the version",
                        stored.len()
                    ),
                });
            }
        }

        self.plugins
            .replace_skills(&current.id, version.clone(), &skills)
            .await
            .map_err(SkillError::Db)?;
        self.checked
            .insert(current.id.clone(), Instant::now())
            .await;

        debug!(
            plugin_id = %current.id,
            skill_count = skills.len(),
            "Plugin synced"
        );
        Ok(())
    }
}

/// A plugin's repository, which is always read as a GitHub one: the `plugins`
/// table records no provider (see `docs/deferred-changes.md`).
pub(super) fn plugin_remote(git_url: &str) -> RepositoryRemote {
    crate::repository::remote::resolve_remote(Some(RepositoryProvider::Github), git_url.to_string())
}

/// `actor`'s own credential for a plugin's repository.
///
/// `None` only when `actor` has not connected the host, so a public plugin
/// still fetches unauthenticated. A credential that exists and cannot be used
/// is an error rather than `None`: GitHub answers an unauthenticated read of a
/// private repository with 404, which reads as a plugin with no skills.
pub(super) async fn plugin_token(
    git_host_tokens: &dyn GitHostTokenIssuer,
    actor: &Actor,
    remote: &RepositoryRemote,
) -> Result<Option<String>, SkillError> {
    let Some(repo) = remote.host_repo.as_ref() else {
        return Ok(None);
    };
    match git_host_tokens
        .token(actor, &repo.credential_account())
        .await
    {
        Ok(token) => Ok(Some(token)),
        Err(GitHostTokenError::NoCredential { .. }) => {
            debug!(
                git_url = %remote.url,
                %actor,
                "No GitHub credential for plugin — fetching unauthenticated"
            );
            Ok(None)
        }
        Err(e) => Err(SkillError::RepositoryToken(
            crate::repository::access::git_host_token_error(e),
        )),
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
        finds_none: bool,
        version_delay: Duration,
    }

    impl CountingFetcher {
        fn at_version(version: &str) -> Self {
            Self {
                version: StdMutex::new(version.to_string()),
                versions_probed: AtomicUsize::new(0),
                files_fetched: AtomicUsize::new(0),
                fail_fetch: false,
                finds_none: false,
                version_delay: Duration::ZERO,
            }
        }

        fn hanging_at(version: &str) -> Self {
            Self {
                version_delay: Duration::from_mins(1),
                ..Self::at_version(version)
            }
        }

        fn finding_no_skills_at(version: &str) -> Self {
            Self {
                finds_none: true,
                ..Self::at_version(version)
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
            if self.finds_none {
                return Ok(Vec::new());
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
            tokio::time::sleep(self.version_delay).await;
            Ok(Some(self.version.lock().unwrap().clone()))
        }

        async fn fetch_archive(
            &self,
            _remote: &RepositoryRemote,
            _version: &str,
            _token: Option<&str>,
        ) -> Result<Vec<u8>, SkillError> {
            unimplemented!()
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
        timed_service(plugins, fetcher, CHECK_INTERVAL, CHECK_TIMEOUT)
    }

    fn timed_service(
        plugins: Arc<dyn PluginDao>,
        fetcher: Arc<dyn SkillFetcher>,
        check_interval: Duration,
        check_timeout: Duration,
    ) -> PluginSyncService {
        PluginSyncService::with_timing(
            plugins,
            fetcher,
            // No credential rows: the token resolves to None and the fetch
            // goes out unauthenticated, which is not what these tests are
            // about.
            moatless_identity::git_host_tokens::seam_over(Arc::new(
                StubProviderTokens::with_user_row(None),
            )),
            check_interval,
            check_timeout,
        )
    }

    fn seeded_snapshot() -> Vec<PluginSkill> {
        vec![PluginSkill {
            plugin_id: "p1".to_string(),
            name: "code-review".to_string(),
            description: "Reviews code".to_string(),
            path: "skills/code-review/SKILL.md".to_string(),
        }]
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
            1,
            "the second call falls inside the check interval and must not ask \
             the source again"
        );
    }

    #[tokio::test]
    async fn a_source_is_checked_again_once_the_interval_has_passed() {
        let plugins = Arc::new(StubPlugins::with(plugin(None)));
        let fetcher = Arc::new(CountingFetcher::at_version("sha-1"));
        let sync = timed_service(
            plugins.clone(),
            fetcher.clone(),
            Duration::ZERO,
            CHECK_TIMEOUT,
        );

        sync.sync(&actor(), &plugin(None)).await.unwrap();
        sync.sync(&actor(), &plugin(None)).await.unwrap();

        assert_eq!(fetcher.versions_probed.load(Ordering::SeqCst), 2);
        assert_eq!(fetcher.files_fetched.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_hanging_update_check_serves_the_stored_snapshot_and_backs_off() {
        let plugins =
            Arc::new(StubPlugins::with(plugin(Some("sha-old"))).seed_skills(seeded_snapshot()));
        let fetcher = Arc::new(CountingFetcher::hanging_at("sha-new"));
        let sync = timed_service(
            plugins.clone(),
            fetcher.clone(),
            CHECK_INTERVAL,
            Duration::from_millis(20),
        );

        let first = sync
            .skills(&actor(), &plugin(Some("sha-old")))
            .await
            .unwrap();
        let second = sync
            .skills(&actor(), &plugin(Some("sha-old")))
            .await
            .unwrap();

        assert_eq!(first, seeded_snapshot());
        assert_eq!(second, seeded_snapshot());
        assert_eq!(
            fetcher.versions_probed.load(Ordering::SeqCst),
            1,
            "a timed-out check counts as a check, so the next read does not \
             wait on the same unreachable host"
        );
        assert_eq!(fetcher.files_fetched.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_plugin_with_nothing_stored_retries_a_failed_sync_at_once() {
        let plugins = Arc::new(StubPlugins::with(plugin(None)));
        let fetcher = Arc::new(CountingFetcher::failing());
        let sync = service(plugins.clone(), fetcher.clone());

        assert!(sync.sync(&actor(), &plugin(None)).await.is_err());
        assert!(sync.sync(&actor(), &plugin(None)).await.is_err());

        assert_eq!(fetcher.versions_probed.load(Ordering::SeqCst), 2);
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
        let previous = seeded_snapshot();
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

    #[tokio::test]
    async fn a_sync_finding_no_skills_keeps_the_stored_ones_and_their_version() {
        let plugins =
            Arc::new(StubPlugins::with(plugin(Some("sha-old"))).seed_skills(seeded_snapshot()));
        let fetcher = Arc::new(CountingFetcher::finding_no_skills_at("sha-new"));
        let sync = service(plugins.clone(), fetcher.clone());

        assert!(sync.sync(&actor(), &plugin(Some("sha-old"))).await.is_err());

        assert_eq!(plugins.list_skills("p1").await.unwrap(), seeded_snapshot());
        assert_eq!(
            plugins
                .find_by_id("p1")
                .await
                .unwrap()
                .unwrap()
                .synced_version,
            Some("sha-old".to_string()),
            "an unrecorded version is what makes the next sync fetch again"
        );
    }

    #[tokio::test]
    async fn a_plugin_with_nothing_stored_may_sync_to_no_skills() {
        let plugins = Arc::new(StubPlugins::with(plugin(None)));
        let sync = service(
            plugins.clone(),
            Arc::new(CountingFetcher::finding_no_skills_at("sha-1")),
        );

        sync.sync(&actor(), &plugin(None)).await.unwrap();

        assert_eq!(
            plugins
                .find_by_id("p1")
                .await
                .unwrap()
                .unwrap()
                .synced_version,
            Some("sha-1".to_string())
        );
    }

    /// Answers as an identity whose GitHub App token expired and could not be
    /// refreshed.
    struct BrokenTokens;

    #[async_trait]
    impl GitHostTokenIssuer for BrokenTokens {
        async fn token(
            &self,
            actor: &Actor,
            _account: &moatless_types::git_host::GitHostAccount,
        ) -> Result<String, GitHostTokenError> {
            Err(GitHostTokenError::Broken {
                actor: actor.clone(),
                provider: RepositoryProvider::Github,
                source: "bad_refresh_token".into(),
            })
        }

        async fn github_is_usable(&self, _actor: &Actor) -> Result<(), GitHostTokenError> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn a_broken_credential_fails_the_sync_instead_of_fetching_unauthenticated() {
        let plugins =
            Arc::new(StubPlugins::with(plugin(Some("sha-old"))).seed_skills(seeded_snapshot()));
        let fetcher = Arc::new(CountingFetcher::finding_no_skills_at("sha-new"));
        let sync = PluginSyncService::new(plugins.clone(), fetcher.clone(), Arc::new(BrokenTokens));

        let err = sync
            .sync(&actor(), &plugin(Some("sha-old")))
            .await
            .unwrap_err();

        assert!(matches!(err, SkillError::RepositoryToken(_)), "{err}");
        assert_eq!(
            fetcher.versions_probed.load(Ordering::SeqCst),
            0,
            "an unauthenticated probe of a private repository answers 404"
        );
        assert_eq!(plugins.list_skills("p1").await.unwrap(), seeded_snapshot());
    }
}
