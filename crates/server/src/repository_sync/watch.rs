//! Automatic Loop and workspace sync when a repository's default branch moves.
//!
//! Each tick syncs the GitHub repositories whose head changed since the last
//! successful sync. An owner whose webhook delivered a default-branch push
//! within [`PUSH_COVERAGE_WINDOW`] is covered: its repositories take their head
//! from the recorded push, and every other repository's head is read from
//! GitHub. Loops land inactive and keep any approval, as in `sync-config`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use moatless_adapter_model::connections::{ConnectionEvent, ConnectionRouting};
use moatless_core::leader::LeaderGuard;
use moatless_loops::loops::sync::LoopSyncService;
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::AdapterKind;
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

/// How long one default-branch push vouches for its owner's webhook.
///
/// An owner with no push this recent is read like any other, so a hook that
/// stops sending `push` falls back to polling without anyone noticing.
pub const PUSH_COVERAGE_WINDOW: Duration = Duration::from_hours(24);

/// How often a covered repository's head is still read, to recover a push
/// whose webhook delivery failed: GitHub does not redeliver on its own.
pub const COVERED_REREAD_INTERVAL: Duration = Duration::from_hours(6);

/// Syncs a repository's committed Loops and workspaces once its default branch
/// has moved.
pub struct DefaultBranchWatch {
    repositories: Arc<Repositories>,
    connections: Arc<ConnectionRouting>,
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
    /// The receipt time of the newest recorded push that `synced_head`
    /// already reflects, so an older push never re-syncs over a newer read.
    pushes_through: Option<DateTime<Utc>>,
    /// Set after an unauthenticated read, per [`UNAUTHENTICATED_POLL_INTERVAL`].
    next_read: Option<Instant>,
    /// When the head was last read from GitHub, per [`COVERED_REREAD_INTERVAL`].
    last_read: Option<Instant>,
    /// The last failure logged at WARN, so a repeat of it logs at DEBUG.
    last_failure: Option<String>,
}

impl DefaultBranchWatch {
    pub fn new(
        repositories: Arc<Repositories>,
        connections: Arc<ConnectionRouting>,
        loop_sync: Arc<LoopSyncService>,
        git_sync: Arc<GitSyncService>,
    ) -> Self {
        Self {
            repositories,
            connections,
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

    /// Sync every GitHub repository whose head moved, stopping between
    /// repositories once `cancel` fires.
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
        let coverage = self.push_coverage().await;
        for repo in repositories.iter().filter(|repo| {
            repo.remote.is_some() && repo.provider == Some(RepositoryProvider::Github)
        }) {
            if cancel.is_cancelled() {
                return;
            }
            let covered = coverage.for_repository(repo);
            let pushed_at = covered.pushed_at();
            let next = self.with_watched(&repo.id, |w| next_step(w, &covered, Instant::now()));
            match next {
                Next::Read => self.sync_if_moved(repo, pushed_at).await,
                Next::Sync(head) => {
                    self.sync_head(repo, &acting_scope(repo), head, pushed_at)
                        .await;
                }
                Next::Skip => {}
            }
        }
    }

    /// The owners whose webhook delivered a default-branch push within
    /// [`PUSH_COVERAGE_WINDOW`], and the head each push reported. A failed read
    /// covers nobody, so every repository is read as before.
    async fn push_coverage(&self) -> PushCoverage {
        let since = chrono::Utc::now()
            - chrono::Duration::from_std(PUSH_COVERAGE_WINDOW).unwrap_or(chrono::Duration::MAX);
        match self
            .connections
            .events_since(
                AdapterKind::Github.as_str(),
                moatless_adapter_github_pr::DEFAULT_BRANCH_PUSH,
                since,
            )
            .await
        {
            Ok(events) => PushCoverage::from_events(&events),
            Err(e) => {
                warn!(error = %e, "Could not read default-branch pushes; reading every repository");
                PushCoverage::default()
            }
        }
    }

    /// Read `repo`'s head and sync if it moved. A successful read reflects
    /// every push received before it, `pushed_at` included.
    async fn sync_if_moved(&self, repo: &Repository, pushed_at: Option<DateTime<Utc>>) {
        if self
            .with_watched(&repo.id, |w| w.next_read)
            .is_some_and(|next| Instant::now() < next)
        {
            return;
        }
        let ctx = acting_scope(repo);
        let head = match self.git_sync.default_branch_head(repo, &ctx).await {
            Ok(head) => head,
            Err(e) => {
                self.failed(repo, format!("could not read the head: {e}"));
                return;
            }
        };
        self.with_watched(&repo.id, |w| {
            let now = Instant::now();
            w.last_read = Some(now);
            w.next_read = (!head.authenticated).then(|| now + UNAUTHENTICATED_POLL_INTERVAL);
        });
        let head = head.sha;
        if self.with_watched(&repo.id, |w| {
            let unchanged = w.synced_head.as_deref() == Some(head.as_str());
            if unchanged {
                w.pushes_through = w.pushes_through.max(pushed_at);
            }
            unchanged
        }) {
            debug!(repository_id = %repo.id, %head, "Default branch unchanged");
            return;
        }
        self.sync_head(repo, &ctx, head, pushed_at).await;
    }

    /// Sync `repo`'s workspaces and Loops, recording `head` and `pushed_at`
    /// once both land.
    async fn sync_head(
        &self,
        repo: &Repository,
        ctx: &ScopeContext,
        head: String,
        pushed_at: Option<DateTime<Utc>>,
    ) {
        // Workspaces first: a Loop resolves its `workspace:` against them. A
        // failure still lets the Loops sync, but leaves the head unrecorded so
        // a Loop naming a workspace this head adds is retried next tick.
        let workspaces = self.git_sync.sync_repo_workspaces_for(repo, ctx).await;

        let summary = match self.loop_sync.sync_repo_loops_for(repo, ctx).await {
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
            w.pushes_through = w.pushes_through.max(pushed_at);
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

/// The scope a background sync of `repo` acts in: its creator's, whose
/// credential reads it and whom `managed_loop_input` records as a synced Loop's
/// creator.
fn acting_scope(repo: &Repository) -> ScopeContext {
    let acting = repo
        .created_by
        .as_deref()
        .or(repo.owner_user_id.as_deref())
        .unwrap_or(ScopeContext::SYSTEM_USER_ID);
    ScopeContext::user(acting)
}

/// What the webhook's recent default-branch pushes say, keyed lowercase:
/// GitHub owner and repository names are case-insensitive.
#[derive(Default)]
struct PushCoverage {
    owners: HashSet<String>,
    /// `owner/repo` to its latest push.
    heads: HashMap<String, Pushed>,
}

impl PushCoverage {
    fn from_events(events: &[ConnectionEvent]) -> Self {
        let mut coverage = Self::default();
        for event in events {
            coverage.owners.insert(event.scope_value.to_lowercase());
            let key = event.subject_key.to_lowercase();
            if coverage
                .heads
                .get(&key)
                .is_none_or(|pushed| pushed.at < event.received_at)
            {
                coverage.heads.insert(
                    key,
                    Pushed {
                        head: event.external_id.clone(),
                        at: event.received_at,
                    },
                );
            }
        }
        coverage
    }

    /// How `repo`'s head is known this tick.
    fn for_repository(&self, repo: &Repository) -> Coverage {
        let Some(host) = repo.remote.as_ref().and_then(|r| r.host_repo.as_ref()) else {
            return Coverage::Polled;
        };
        let owner = host.owner().to_lowercase();
        if !self.owners.contains(&owner) {
            return Coverage::Polled;
        }
        let key = format!("{owner}/{}", host.name().to_lowercase());
        Coverage::Webhook {
            pushed: self.heads.get(&key).cloned(),
        }
    }
}

/// A recorded default-branch push.
#[derive(Clone, Debug)]
struct Pushed {
    head: String,
    /// When the webhook delivered it, on the database's clock.
    at: DateTime<Utc>,
}

/// How a repository's head is known.
#[derive(Debug)]
enum Coverage {
    /// Its owner's webhook delivered no recent push, so GitHub is read.
    Polled,
    /// Its owner's webhook delivers pushes; `pushed` is this repository's
    /// latest push in [`PUSH_COVERAGE_WINDOW`], if it was pushed to.
    Webhook { pushed: Option<Pushed> },
}

impl Coverage {
    fn pushed_at(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Webhook {
                pushed: Some(pushed),
            } => Some(pushed.at),
            _ => None,
        }
    }
}

/// What a tick does for one repository.
#[derive(Debug, PartialEq, Eq)]
enum Next {
    /// Read the head from GitHub, and sync if it moved.
    Read,
    /// Sync to this pushed head without reading GitHub.
    Sync(String),
    Skip,
}

/// Decide [`Next`] for a repository, given `covered` as
/// [`PushCoverage::for_repository`] reports it.
fn next_step(watched: &Watched, covered: &Coverage, now: Instant) -> Next {
    let Coverage::Webhook { pushed } = covered else {
        return Next::Read;
    };
    // First sight since start: a push may have landed while no replica was up.
    if watched.synced_head.is_none()
        || watched
            .last_read
            .is_none_or(|read| now.duration_since(read) >= COVERED_REREAD_INTERVAL)
    {
        return Next::Read;
    }
    match pushed {
        // Compared by receipt time rather than sha: a re-read can have synced
        // past the recorded push, and syncing back to it would repeat a full
        // sync every tick.
        Some(pushed)
            if watched
                .pushes_through
                .is_none_or(|through| pushed.at > through)
                && watched.synced_head.as_deref() != Some(pushed.head.as_str()) =>
        {
            Next::Sync(pushed.head.clone())
        }
        _ => Next::Skip,
    }
}

#[cfg(test)]
mod tests {
    use super::{COVERED_REREAD_INTERVAL, Coverage, Next, Pushed, Watched, next_step};
    use chrono::{DateTime, Utc};
    use std::time::Instant;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).unwrap_or_default()
    }

    fn pushed(push: Option<(&str, i64)>) -> Coverage {
        Coverage::Webhook {
            pushed: push.map(|(head, seconds)| Pushed {
                head: head.to_string(),
                at: at(seconds),
            }),
        }
    }

    fn watched(synced: Option<&str>, read: Option<Instant>) -> Watched {
        Watched {
            synced_head: synced.map(str::to_string),
            last_read: read,
            ..Watched::default()
        }
    }

    #[test]
    fn an_uncovered_repository_is_read_every_tick() {
        let now = Instant::now();
        assert_eq!(
            next_step(&watched(Some("a"), Some(now)), &Coverage::Polled, now),
            Next::Read
        );
    }

    #[test]
    fn a_covered_repository_syncs_to_a_new_pushed_head_without_reading() {
        let now = Instant::now();
        let synced = watched(Some("a"), Some(now));
        assert_eq!(
            next_step(&synced, &pushed(Some(("b", 10))), now),
            Next::Sync("b".into())
        );
        assert_eq!(
            next_step(&synced, &pushed(Some(("a", 10))), now),
            Next::Skip
        );
        assert_eq!(next_step(&synced, &pushed(None), now), Next::Skip);
    }

    #[test]
    fn a_push_older_than_what_the_head_reflects_is_not_synced_back_to() {
        let now = Instant::now();
        let reread = Watched {
            pushes_through: Some(at(10)),
            ..watched(Some("x"), Some(now))
        };
        assert_eq!(
            next_step(&reread, &pushed(Some(("p", 10))), now),
            Next::Skip
        );
        assert_eq!(
            next_step(&reread, &pushed(Some(("q", 11))), now),
            Next::Sync("q".into())
        );
    }

    #[test]
    fn a_covered_repository_is_still_read_once_at_start_and_then_rarely() {
        let now = Instant::now();
        assert_eq!(
            next_step(&watched(None, None), &pushed(Some(("b", 10))), now),
            Next::Read
        );
        let stale = now
            .checked_sub(COVERED_REREAD_INTERVAL)
            .expect("monotonic clock");
        assert_eq!(
            next_step(&watched(Some("a"), Some(stale)), &pushed(None), now),
            Next::Read
        );
    }
}
