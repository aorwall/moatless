//! Reading a stopped task's workspace out of its `restic` repository.
//!
//! One repository per task, under `<prefix>/<task_id>/repo`, opened with the
//! password derived from the deployment's master secret. The sandbox writes it
//! (`crates/sandbox/src/workspace_sync/restic.rs`); this is the read half.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moatless_core::workspace_repository::derive_repository_password;
use moatless_ports::sandbox::{
    SnapshotError, SnapshotFile, SnapshotListing, WorkspaceSnapshotSource,
};
use moatless_s3::ResticS3Backend;
use moatless_sandbox_runtime::storage::{SNAPSHOT_ROOT, workspace_repository_prefix};
use rustic_core::repofile::{NodeType, SnapshotFile as ResticSnapshot};
use rustic_core::{
    Credentials, IndexedFullStatus, LsOptions, Repository, RepositoryBackends, RepositoryOptions,
};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use super::tree::{SnapshotNode, dir_prefix, listing_under, resolve, search_under};

/// How long an open repository and its file set are reused. Matches the
/// mirror's manifest TTL: a running task can snapshot again while one is held,
/// so this bounds how long a listing can describe the previous generation.
const REPOSITORY_TTL: Duration = Duration::from_secs(30);

/// Cache slots held at once. An indexed repository holds its whole pack index
/// in memory, which for a workspace-sized tree is the same order as the
/// mirror's manifest, so the two caps are kept equal. [`Entry::Absent`] slots
/// share the budget and cost nothing to rebuild, so a fleet of mirror-format
/// tasks can evict a live index — which costs one repeated open, never a wrong
/// answer.
const REPOSITORY_CACHE_CAP: usize = 32;

/// Where the local pack cache lives.
///
/// Mandatory, never `no_cache`: without it `rustic_core` issues one range GET
/// per blob, which measured in-cluster as a tree listing going from 628 ms to
/// 15.6 s and a restore from 13 requests to 1,222.
const CACHE_DIR: &str = "/tmp/rustic-cache";

struct Cached {
    repo: Arc<Repository<IndexedFullStatus>>,
    /// The snapshot `nodes` was walked from. Reads address through it rather
    /// than re-resolving `latest`, so a snapshot written inside the TTL cannot
    /// move a read onto trees the held index does not cover.
    snapshot: Arc<ResticSnapshot>,
    nodes: Arc<Vec<SnapshotNode>>,
    synced_at: DateTime<Utc>,
    fetched: Instant,
}

/// A cache slot. `Absent` is held so a task on the file mirror does not probe
/// `config` again on every read, list and search.
enum Entry {
    Open(Cached),
    Absent(Instant),
}

impl Entry {
    fn fetched(&self) -> Instant {
        match self {
            Self::Open(cached) => cached.fetched,
            Self::Absent(at) => *at,
        }
    }
}

/// [`WorkspaceSnapshotSource`] over the per-task `restic` repository.
pub struct ResticWorkspaceSnapshots {
    client: aws_sdk_s3::Client,
    bucket: String,
    prefix: String,
    master_secret: String,
    cache: Mutex<HashMap<String, Entry>>,
    /// One lock per task, so concurrent misses pay the open once. A file tree
    /// opening several files at a time is exactly that shape, and each open is
    /// the ~400 ms scrypt derivation below.
    opening: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl ResticWorkspaceSnapshots {
    pub fn new(
        client: aws_sdk_s3::Client,
        bucket: String,
        prefix: String,
        master_secret: String,
    ) -> Self {
        Self {
            client,
            bucket,
            prefix,
            master_secret,
            cache: Mutex::new(HashMap::new()),
            opening: Mutex::new(HashMap::new()),
        }
    }

    /// The task's open repository, file set, and the time of its latest
    /// snapshot — from cache when fresh.
    ///
    /// Opening is what this cache exists for: the scrypt key derivation in
    /// `Repository::open` costs about 400 ms of CPU on a sandbox node, which is
    /// 85% of a cold read that already hits a populated pack cache, and it does
    /// not shrink on a faster network.
    async fn opened(&self, task_id: &str) -> Result<Cached, SnapshotError> {
        if let Some(hit) = self.cached(task_id).await {
            return hit;
        }

        // One opener per task. Whoever loses the race re-checks the cache and
        // finds the winner's entry rather than deriving the key a second time.
        let gate = {
            let mut opening = self.opening.lock().await;
            // A gate the map alone holds has no opener behind it. Without this
            // the map grows one permanent entry per task ever browsed — the
            // same axis `REPOSITORY_CACHE_CAP` bounds the cache on.
            opening.retain(|_, gate| Arc::strong_count(gate) > 1);
            Arc::clone(
                opening
                    .entry(task_id.to_string())
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
        let _opening = gate.lock().await;
        if let Some(hit) = self.cached(task_id).await {
            return hit;
        }

        let backend = ResticS3Backend::new(
            self.client.clone(),
            self.bucket.clone(),
            &workspace_repository_prefix(&self.prefix, task_id),
        );
        let password = derive_repository_password(&self.master_secret, task_id);

        // `rustic_core` is synchronous throughout, so the whole open and tree
        // walk happens inside one `spawn_blocking` and no repository value is
        // ever held across an `.await`.
        let opened = tokio::task::spawn_blocking(move || open_and_walk(&backend, &password))
            .await
            .map_err(|err| {
                SnapshotError::Unavailable(format!("repository read panicked: {err}"))
            })?;

        let opened = match opened {
            Ok(opened) => opened,
            Err(SnapshotError::NoSnapshot) => {
                self.store(task_id, Entry::Absent(Instant::now())).await;
                return Err(SnapshotError::NoSnapshot);
            }
            Err(other) => return Err(other),
        };

        let (repo, snapshot, nodes, synced_at) = opened;
        debug!(task_id = %task_id, files = nodes.len(), "Opened a workspace restic repository");
        let cached = Cached {
            repo,
            snapshot,
            nodes,
            synced_at,
            fetched: Instant::now(),
        };
        let handle = cached.clone_handle();
        self.store(task_id, Entry::Open(cached)).await;
        Ok(handle)
    }

    /// A fresh cache entry for the task, or `None` when it has to be opened.
    async fn cached(&self, task_id: &str) -> Option<Result<Cached, SnapshotError>> {
        let cache = self.cache.lock().await;
        match cache.get(task_id) {
            Some(entry) if entry.fetched().elapsed() < REPOSITORY_TTL => match entry {
                Entry::Open(cached) => Some(Ok(cached.clone_handle())),
                Entry::Absent(_) => Some(Err(SnapshotError::NoSnapshot)),
            },
            _ => None,
        }
    }

    async fn store(&self, task_id: &str, entry: Entry) {
        let mut cache = self.cache.lock().await;
        cache.retain(|_, held| held.fetched().elapsed() < REPOSITORY_TTL);
        if cache.len() >= REPOSITORY_CACHE_CAP {
            if let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, held)| held.fetched())
                .map(|(id, _)| id.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(task_id.to_string(), entry);
    }
}

impl Cached {
    fn clone_handle(&self) -> Self {
        Self {
            repo: Arc::clone(&self.repo),
            snapshot: Arc::clone(&self.snapshot),
            nodes: Arc::clone(&self.nodes),
            synced_at: self.synced_at,
            fetched: self.fetched,
        }
    }
}

type Opened = (
    Arc<Repository<IndexedFullStatus>>,
    Arc<ResticSnapshot>,
    Arc<Vec<SnapshotNode>>,
    DateTime<Utc>,
);

/// Open the repository and walk its latest snapshot into a flat file set.
fn open_and_walk(backend: &ResticS3Backend, password: &str) -> Result<Opened, SnapshotError> {
    let backends = RepositoryBackends::new(Arc::new(backend.clone()), None);
    let options = RepositoryOptions::default().cache_dir(CACHE_DIR);
    let repo = Repository::new(&options, &backends)
        .map_err(|err| SnapshotError::Unavailable(format!("repository is unusable: {err}")))?;

    // A missing `config` object is a task that has never snapshotted, not an
    // outage. The port is explicit that the two must not read alike.
    match repo.config_id() {
        Ok(None) => return Err(SnapshotError::NoSnapshot),
        Ok(Some(_)) => {}
        Err(err) => {
            return Err(SnapshotError::Unavailable(format!(
                "repository config is unreadable: {err}"
            )));
        }
    }

    let repo = repo
        .open(&Credentials::password(password))
        .map_err(|err| SnapshotError::Unavailable(format!("repository will not open: {err}")))?
        .to_indexed()
        .map_err(|err| {
            SnapshotError::Unavailable(format!("repository index is unusable: {err}"))
        })?;

    let snapshot = match repo.get_snapshot_from_str("latest", |_| true) {
        Ok(snapshot) => snapshot,
        // A repository with a `config` but no snapshot is a sandbox that
        // initialised the repository and was stopped before its first sync.
        Err(err) => {
            debug!(error = %err, "Workspace repository holds no snapshot yet");
            return Err(SnapshotError::NoSnapshot);
        }
    };
    let synced_at = DateTime::from_timestamp(
        snapshot.time.timestamp().as_second(),
        snapshot.time.timestamp().subsec_nanosecond().unsigned_abs(),
    )
    .unwrap_or_else(|| {
        warn!(time = %snapshot.time, "Snapshot timestamp will not convert; reporting now");
        Utc::now()
    });

    let root = repo
        .node_from_snapshot_and_path(&snapshot, SNAPSHOT_ROOT)
        .map_err(|err| {
            SnapshotError::Unavailable(format!("snapshot has no `{SNAPSHOT_ROOT}` tree: {err}"))
        })?;

    let mut nodes = Vec::new();
    let streamer = repo
        .ls(&root, &LsOptions::default())
        .map_err(|err| SnapshotError::Unavailable(format!("snapshot tree is unwalkable: {err}")))?;
    for item in streamer {
        let (path, node) = item
            .map_err(|err| SnapshotError::Unavailable(format!("snapshot tree is broken: {err}")))?;
        // Directories are dropped: the listing synthesises them from file paths
        // so this format and the mirror produce identical trees.
        let symlink = match &node.node_type {
            NodeType::Dir => continue,
            NodeType::Symlink { linktarget, .. } => Some(linktarget.clone()),
            _ => None,
        };
        nodes.push(SnapshotNode {
            path: path.to_string_lossy().into_owned(),
            size: node.meta.size,
            symlink,
        });
    }

    Ok((
        Arc::new(repo),
        Arc::new(snapshot),
        Arc::new(nodes),
        synced_at,
    ))
}

#[async_trait]
impl WorkspaceSnapshotSource for ResticWorkspaceSnapshots {
    /// No path-safety check, unlike the mirror's: `resolve` only ever returns a
    /// path already present in the walked node set, so a caller cannot address
    /// anything the snapshot does not hold.
    async fn read(&self, task_id: &str, path: &str) -> Result<SnapshotFile, SnapshotError> {
        let held = self.opened(task_id).await?;
        let stored = resolve(&held.nodes, path.trim_start_matches('/'))
            .ok_or(SnapshotError::NotFound)?
            .to_string();

        let repo = Arc::clone(&held.repo);
        let snapshot = Arc::clone(&held.snapshot);
        let synced_at = held.synced_at;
        tokio::task::spawn_blocking(move || {
            // `resolve` already found the path in this snapshot's node set, so
            // a failure here is the repository, not a missing file. Reporting
            // it as `NotFound` would send an outage to the caller as an answer:
            // the chain stops looking and `t3code` logs nothing.
            let node = repo
                .node_from_snapshot_and_path(&snapshot, &format!("{SNAPSHOT_ROOT}/{stored}"))
                .map_err(|err| {
                    SnapshotError::Unavailable(format!("file `{stored}` is unresolvable: {err}"))
                })?;
            let file = repo.open_file(&node).map_err(|err| {
                SnapshotError::Unavailable(format!("file `{stored}` will not open: {err}"))
            })?;
            let content = repo
                .read_file_at(&file, 0, node.meta.size as usize)
                .map_err(|err| {
                    SnapshotError::Unavailable(format!("file `{stored}` is unreadable: {err}"))
                })?;
            Ok(SnapshotFile {
                content: content.to_vec(),
                synced_at,
            })
        })
        .await
        .map_err(|err| SnapshotError::Unavailable(format!("file read panicked: {err}")))?
    }

    async fn list(
        &self,
        task_id: &str,
        path: &str,
        recursive: bool,
    ) -> Result<SnapshotListing, SnapshotError> {
        let held = self.opened(task_id).await?;
        Ok(SnapshotListing {
            entries: listing_under(&held.nodes, &dir_prefix(path), recursive),
            synced_at: held.synced_at,
        })
    }

    async fn search(
        &self,
        task_id: &str,
        query: &str,
        case_sensitive: bool,
        limit: usize,
    ) -> Result<SnapshotListing, SnapshotError> {
        let held = self.opened(task_id).await?;
        Ok(SnapshotListing {
            entries: search_under(&held.nodes, query, case_sensitive, limit),
            synced_at: held.synced_at,
        })
    }
}
