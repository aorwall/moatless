//! Plugin files served to sandboxes, cached by commit.
//!
//! A sandbox fetches every plugin it runs through the backend rather than
//! cloning it, so a fleet of starts costs the git host one version probe each
//! and one download per commit per backend replica. The cache holds only
//! [`plugin_files`], never the repository archive it came from.

use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;
use tracing::debug;

use super::files::plugin_files;
use super::sync::{plugin_remote, plugin_token};
use crate::skill::error::SkillError;
use crate::skill::fetcher::SkillFetcher;
use moatless_ports::identity::GitHostTokenIssuer;
use moatless_types::access_control::Actor;

/// Bytes of archive held across all plugins before the least used are evicted.
const CACHE_CAPACITY_BYTES: u64 = 512 * 1024 * 1024;

/// One plugin's files at a commit, as a gzip tarball rooted at the repository
/// root.
#[derive(Debug, Clone)]
pub struct PluginArchive {
    /// The commit the archive was taken at.
    pub version: String,
    pub data: Arc<Vec<u8>>,
}

/// Downloads plugin archives and keeps them keyed by repository and commit.
pub struct PluginArchives {
    fetcher: Arc<dyn SkillFetcher>,
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    archives: Cache<(String, String), Arc<Vec<u8>>>,
}

impl PluginArchives {
    pub(crate) fn new(
        fetcher: Arc<dyn SkillFetcher>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    ) -> Self {
        Self {
            fetcher,
            git_host_tokens,
            archives: Cache::builder()
                .weigher(|_, data: &Arc<Vec<u8>>| u32::try_from(data.len()).unwrap_or(u32::MAX))
                .max_capacity(CACHE_CAPACITY_BYTES)
                .time_to_idle(Duration::from_hours(24))
                .build(),
        }
    }

    /// The plugin at `git_url`'s default branch head, read as `actor`, or
    /// `None` when that head is still `known_version`.
    ///
    /// The version probe runs as `actor` on every call, cached archive or not:
    /// it is how a new commit is noticed, and what refuses an `actor` who
    /// cannot read the repository a copy from another identity is cached for.
    pub async fn archive(
        &self,
        actor: &Actor,
        git_url: &str,
        known_version: Option<&str>,
    ) -> Result<Option<PluginArchive>, SkillError> {
        let remote = plugin_remote(git_url);
        let token = plugin_token(self.git_host_tokens.as_ref(), actor, &remote).await;
        let version = self
            .fetcher
            .fetch_version(&remote, token.as_deref())
            .await?
            .ok_or_else(|| SkillError::Fetch {
                git_url: git_url.to_string(),
                message: "the host reported no commit to pin the archive to".to_string(),
            })?;
        if known_version == Some(version.as_str()) {
            return Ok(None);
        }

        // Concurrent callers for one key share a single download.
        let data = self
            .archives
            .try_get_with((git_url.to_string(), version.clone()), async {
                debug!(git_url, version = %version, "Downloading plugin archive");
                let tarball = self
                    .fetcher
                    .fetch_archive(&remote, &version, token.as_deref())
                    .await?;
                tokio::task::spawn_blocking(move || plugin_files(&tarball))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|files| files.map_err(|e| e.to_string()))
                    .map(Arc::new)
                    .map_err(|message| SkillError::Fetch {
                        git_url: git_url.to_string(),
                        message: format!("unreadable plugin archive: {message}"),
                    })
            })
            .await
            .map_err(|shared| {
                Arc::try_unwrap(shared).unwrap_or_else(|shared| SkillError::Fetch {
                    git_url: git_url.to_string(),
                    message: shared.to_string(),
                })
            })?;
        Ok(Some(PluginArchive { version, data }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;

    use super::*;
    use crate::skill::fetcher::{FetchedSkillFile, SkillLayout};
    use moatless_identity::auth::test_support::StubProviderTokens;
    use moatless_workspace_types::repository::RepositoryRemote;

    const GIT_URL: &str = "https://github.com/acme/plugins";

    /// Answers `version` to every probe, or refuses it, and counts downloads.
    struct StubFetcher {
        version: Mutex<Option<String>>,
        downloads: AtomicUsize,
    }

    impl StubFetcher {
        fn at(version: &str) -> Self {
            Self {
                version: Mutex::new(Some(version.to_string())),
                downloads: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl SkillFetcher for StubFetcher {
        async fn fetch_skill_files(
            &self,
            _remote: &RepositoryRemote,
            _layout: SkillLayout,
            _token: Option<&str>,
        ) -> Result<Vec<FetchedSkillFile>, SkillError> {
            unimplemented!()
        }

        async fn fetch_version(
            &self,
            remote: &RepositoryRemote,
            _token: Option<&str>,
        ) -> Result<Option<String>, SkillError> {
            match self.version.lock().unwrap().clone() {
                Some(version) => Ok(Some(version)),
                None => Err(SkillError::AuthRequired {
                    git_url: remote.url.clone(),
                }),
            }
        }

        async fn fetch_archive(
            &self,
            _remote: &RepositoryRemote,
            version: &str,
            _token: Option<&str>,
        ) -> Result<Vec<u8>, SkillError> {
            self.downloads.fetch_add(1, Ordering::SeqCst);
            // Slow enough that a concurrent caller queues on the same key.
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(host_archive(version))
        }
    }

    /// A GitHub-shaped tarball whose one skill's body is `version`.
    fn host_archive(version: &str) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_size(version.len() as u64);
        builder
            .append_data(
                &mut header,
                "acme-abc/skills/x/SKILL.md",
                version.as_bytes(),
            )
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn skill_body(archive: &PluginArchive) -> String {
        use std::io::Read;
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive.data.as_slice()));
        let mut entry = tar.entries().unwrap().next().unwrap().unwrap();
        let mut body = String::new();
        entry.read_to_string(&mut body).unwrap();
        body
    }

    fn archives(fetcher: Arc<StubFetcher>) -> PluginArchives {
        PluginArchives::new(
            fetcher,
            moatless_identity::git_host_tokens::seam_over(Arc::new(
                StubProviderTokens::with_user_row(None),
            )),
        )
    }

    #[tokio::test]
    async fn an_unchanged_commit_is_downloaded_once() {
        let fetcher = Arc::new(StubFetcher::at("sha-1"));
        let archives = archives(fetcher.clone());

        let (u1, u2) = (Actor::of("u1"), Actor::of("u2"));
        let (a, b) = tokio::join!(
            archives.archive(&u1, GIT_URL, None),
            archives.archive(&u2, GIT_URL, None)
        );
        let c = archives.archive(&u1, GIT_URL, None).await.unwrap().unwrap();

        assert_eq!(a.unwrap().unwrap().version, "sha-1");
        assert_eq!(skill_body(&b.unwrap().unwrap()), "sha-1");
        assert_eq!(skill_body(&c), "sha-1");
        assert_eq!(fetcher.downloads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_new_commit_is_downloaded_again() {
        let fetcher = Arc::new(StubFetcher::at("sha-1"));
        let archives = archives(fetcher.clone());
        archives
            .archive(&Actor::of("u1"), GIT_URL, None)
            .await
            .unwrap();

        *fetcher.version.lock().unwrap() = Some("sha-2".to_string());
        let archive = archives
            .archive(&Actor::of("u1"), GIT_URL, Some("sha-1"))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(archive.version, "sha-2");
        assert_eq!(skill_body(&archive), "sha-2");
        assert_eq!(fetcher.downloads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_caller_already_at_the_head_gets_nothing_to_download() {
        let fetcher = Arc::new(StubFetcher::at("sha-1"));
        let archives = archives(fetcher.clone());

        let archive = archives
            .archive(&Actor::of("u1"), GIT_URL, Some("sha-1"))
            .await
            .unwrap();

        assert!(archive.is_none());
        assert_eq!(fetcher.downloads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_cached_archive_is_refused_to_a_caller_the_host_refuses() {
        let fetcher = Arc::new(StubFetcher::at("sha-1"));
        let archives = archives(fetcher.clone());
        archives
            .archive(&Actor::of("u1"), GIT_URL, None)
            .await
            .unwrap();

        *fetcher.version.lock().unwrap() = None;
        let refused = archives.archive(&Actor::of("u2"), GIT_URL, None).await;

        assert!(matches!(refused, Err(SkillError::AuthRequired { .. })));
    }
}
