//! Automatic Loop and workspace sync when a repository's default branch moves.
//!
//! Each tick reads every GitHub repository's default-branch head and syncs the
//! repositories whose head changed since the last successful sync. Loops land
//! inactive and keep any approval, as in the explicit `sync-config`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moatless_core::leader::LeaderGuard;
use moatless_loops::loops::sync::LoopSyncService;
use moatless_types::access_control::ScopeContext;
use moatless_workspace_types::repository::RepositoryProvider;
use moatless_workspaces::repository::{Repositories, models::Repository};
use moatless_workspaces::workspace::git_sync::GitSyncService;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// How often every repository's default-branch head is read: one GitHub call
/// per repository per tick, so a merged `LOOP.md` lands within this window.
pub const DEFAULT_BRANCH_POLL_INTERVAL: Duration = Duration::from_mins(2);

/// How long a repository read without a credential waits before its next
/// read: GitHub allows 60 unauthenticated requests an hour per IP, shared with
/// every other unauthenticated read the backend makes.
pub const UNAUTHENTICATED_POLL_INTERVAL: Duration = Duration::from_mins(30);

/// Syncs a repository's committed Loops and workspaces once its default branch
/// has moved.
pub struct DefaultBranchWatch {
    repositories: Arc<Repositories>,
    loop_sync: Arc<LoopSyncService>,
    git_sync: Arc<GitSyncService>,
    /// Keyed by repository id. In memory, so a restart or a leadership change
    /// syncs every repository once.
    watched: Mutex<HashMap<String, Watched>>,
}

#[derive(Default)]
struct Watched {
    /// The head the last fully successful sync read.
    synced_head: Option<String>,
    /// Set after an unauthenticated read, per [`UNAUTHENTICATED_POLL_INTERVAL`].
    next_read: Option<Instant>,
    /// The last failure logged at WARN, so a repeat of it logs at DEBUG.
    last_failure: Option<String>,
}

impl DefaultBranchWatch {
    pub fn new(
        repositories: Arc<Repositories>,
        loop_sync: Arc<LoopSyncService>,
        git_sync: Arc<GitSyncService>,
    ) -> Self {
        Self {
            repositories,
            loop_sync,
            git_sync,
            watched: Mutex::new(HashMap::new()),
        }
    }

    /// Spawn the poll loop. A `None` leader syncs on every replica, for a
    /// single-replica Docker deployment.
    pub fn spawn(
        self,
        leader: Option<Arc<dyn LeaderGuard>>,
        interval: Duration,
        cancel: CancellationToken,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!("Default-branch Loop sync started");
            loop {
                tokio::select! {
                    () = cancel.cancelled() => return,
                    () = tokio::time::sleep(interval) => {}
                }
                if let Some(guard) = &leader {
                    match guard.try_acquire_or_renew().await {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(e) => {
                            warn!(error = %e, "Default-branch Loop sync leader election failed");
                            continue;
                        }
                    }
                }
                self.tick(&cancel).await;
            }
        })
    }

    /// Read every GitHub repository's head and sync each one that moved,
    /// stopping between repositories once `cancel` fires.
    pub async fn tick(&self, cancel: &CancellationToken) {
        // reason: a background sync acts for no caller and reads every repository.
        #[allow(clippy::disallowed_methods)]
        let system = ScopeContext::system();
        let repositories = match self.repositories.list_all(&system).await {
            Ok(repositories) => repositories,
            Err(e) => {
                warn!(error = %e, "Default-branch Loop sync could not list repositories");
                return;
            }
        };
        for repo in repositories.iter().filter(|repo| {
            repo.remote.is_some() && repo.provider == Some(RepositoryProvider::Github)
        }) {
            if cancel.is_cancelled() {
                return;
            }
            self.sync_if_moved(repo).await;
        }
    }

    async fn sync_if_moved(&self, repo: &Repository) {
        if self
            .with_watched(&repo.id, |w| w.next_read)
            .is_some_and(|next| Instant::now() < next)
        {
            return;
        }
        // The repository's creator is whose credential reads it, and whom
        // `managed_loop_input` already records as a synced Loop's creator.
        let acting = repo
            .created_by
            .as_deref()
            .or(repo.owner_user_id.as_deref())
            .unwrap_or(ScopeContext::SYSTEM_USER_ID);
        let ctx = ScopeContext::user(acting);

        let head = match self.git_sync.default_branch_head(repo, &ctx).await {
            Ok(head) => head,
            Err(e) => {
                self.failed(repo, format!("could not read the head: {e}"));
                return;
            }
        };
        self.with_watched(&repo.id, |w| {
            w.next_read =
                (!head.authenticated).then(|| Instant::now() + UNAUTHENTICATED_POLL_INTERVAL);
        });
        let head = head.sha;
        if self.with_watched(&repo.id, |w| {
            w.synced_head.as_deref() == Some(head.as_str())
        }) {
            debug!(repository_id = %repo.id, %head, "Default branch unchanged");
            return;
        }

        // Workspaces first: a Loop resolves its `workspace:` against them. A
        // failure still lets the Loops sync, but leaves the head unrecorded so
        // a Loop naming a workspace this head adds is retried next tick.
        let workspaces = self.git_sync.sync_repo_workspaces_for(repo, &ctx).await;

        let summary = match self.loop_sync.sync_repo_loops_for(repo, &ctx).await {
            Ok(summary) => summary,
            Err(e) => {
                let workspace_failure = workspaces
                    .err()
                    .map(|e| format!("; workspace sync failed too: {e}"))
                    .unwrap_or_default();
                self.failed(repo, format!("Loop sync failed: {e}{workspace_failure}"));
                return;
            }
        };
        for skip in &summary.skipped {
            warn!(repository_id = %repo.id, loop_name = %skip.loop_name, reason = %skip.reason, "Default-branch Loop sync skipped a Loop");
        }
        for warning in &summary.warnings {
            warn!(repository_id = %repo.id, loop_name = %warning.loop_name, field = %warning.field, message = %warning.message, "Default-branch Loop sync warning");
        }
        info!(
            repository_id = %repo.id,
            %head,
            created = summary.created,
            updated = summary.updated,
            removed = summary.removed,
            skipped = summary.skipped.len(),
            "Default-branch Loop sync complete"
        );
        if let Err(e) = workspaces {
            self.failed(repo, format!("workspace sync failed: {e}"));
            return;
        }
        self.with_watched(&repo.id, |w| {
            w.synced_head = Some(head);
            w.last_failure = None;
        });
    }

    /// Log a failure that leaves the head unrecorded, at WARN unless it
    /// repeats the last one logged for this repository.
    fn failed(&self, repo: &Repository, failure: String) {
        let repeated = self.with_watched(&repo.id, |w| {
            let repeated = w.last_failure.as_deref() == Some(failure.as_str());
            w.last_failure = Some(failure.clone());
            repeated
        });
        if repeated {
            debug!(repository_id = %repo.id, %failure, "Default-branch Loop sync still failing; Loops left as they are");
        } else {
            warn!(repository_id = %repo.id, %failure, "Default-branch Loop sync failed; Loops left as they are, retried next tick");
        }
    }

    fn with_watched<T>(&self, repository_id: &str, f: impl FnOnce(&mut Watched) -> T) -> T {
        let mut watched = self
            .watched
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(watched.entry(repository_id.to_string()).or_default())
    }
}
