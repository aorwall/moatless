use async_trait::async_trait;
use chrono::DateTime;
use chrono::Utc;

/// Why a snapshot answered nothing.
#[derive(Debug)]
pub enum SnapshotError {
    /// No snapshot is stored for this task, so there is nothing to fall back
    /// to. A task whose sandbox has never synced, and every task that only ever
    /// wrote a tar.
    NoSnapshot,
    /// The snapshot exists and does not hold that path.
    NotFound,
    /// The store could not be read. Distinct from [`Self::NoSnapshot`] because
    /// an outage must not read to a caller as "this task has no files".
    Unavailable(String),
}

/// One file, as the snapshot stored it.
#[derive(Debug, Clone)]
pub struct SnapshotFile {
    pub content: Vec<u8>,
    /// When the snapshot this came from was written.
    pub synced_at: DateTime<Utc>,
}

/// One entry in a snapshot listing. A directory is implied by the paths under
/// it — the snapshot stores files — so `is_dir` marks one this listing
/// synthesised.
#[derive(Debug, Clone)]
pub struct SnapshotEntry {
    /// Workspace-relative path, `/`-separated.
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct SnapshotListing {
    pub entries: Vec<SnapshotEntry>,
    pub synced_at: DateTime<Utc>,
}

/// The workspace as S3 last stored it, for a task whose sandbox is not running.
///
/// The three reads a stopped sandbox can still answer. There is no write: a
/// snapshot is what the workspace was, and changing it would put the store and
/// the next pod's restore at odds.
///
/// **Every method is scoped to one task**, so a caller that has already decided
/// the viewer may read that task has decided the whole access question.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait WorkspaceSnapshotSource: Send + Sync {
    async fn read(&self, task_id: &str, path: &str) -> Result<SnapshotFile, SnapshotError>;

    /// Entries directly under `path`, or the whole subtree when `recursive`.
    async fn list(
        &self,
        task_id: &str,
        path: &str,
        recursive: bool,
    ) -> Result<SnapshotListing, SnapshotError>;

    /// Files whose path contains `query`. `limit` of 0 means no cap.
    async fn search(
        &self,
        task_id: &str,
        query: &str,
        case_sensitive: bool,
        limit: usize,
    ) -> Result<SnapshotListing, SnapshotError>;
}
