//! Reading a stopped task's workspace out of the S3 file mirror.
//!
//! The mirror (`crates/sandbox/src/workspace_sync/mirror.rs`) stores one object
//! per file plus a `manifest.json.gz` naming them all, which is what makes a
//! single file reachable without unpacking anything — and therefore without a
//! sandbox. This is the read half.
//!
//! **Only the mirror.** A task that only ever wrote `workspace.tar.gz` has no
//! manifest and answers [`SnapshotError::NoSnapshot`]: serving one file out of a
//! tar means fetching and scanning the whole archive per request, which is the
//! cost the mirror exists to remove.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moatless_ports::sandbox::{
    SnapshotError, SnapshotFile, SnapshotListing, WorkspaceSnapshotSource,
};
use moatless_sandbox_runtime::storage::{manifest_key, workspace_object_key};
use sandbox_proto::is_safe_workspace_path;
use sandbox_proto::workspace_manifest::{MANIFEST_VERSION, Manifest};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use super::tree::{SnapshotNode, dir_prefix, listing_under, resolve, search_under};

/// How long a fetched manifest is reused.
///
/// A stopped sandbox's mirror does not change, but a task can be started and
/// sync again while a manifest is held, so this bounds how long a listing can
/// describe the previous generation. Short enough that a restarted task's first
/// listing is current, long enough that walking a tree costs one fetch.
const MANIFEST_TTL: Duration = Duration::from_secs(30);

/// Manifests held at once. A manifest is a few hundred KB for a large repo, so
/// this bounds the cache at tens of MB rather than at however many tasks were
/// browsed.
const MANIFEST_CACHE_CAP: usize = 32;

struct Cached {
    nodes: Arc<Vec<SnapshotNode>>,
    synced_at: DateTime<Utc>,
    fetched: Instant,
}

/// [`WorkspaceSnapshotSource`] over the S3 workspace mirror.
pub struct S3WorkspaceSnapshots {
    s3: Arc<moatless_s3::S3Storage>,
    bucket: String,
    prefix: String,
    cache: Mutex<HashMap<String, Cached>>,
}

impl S3WorkspaceSnapshots {
    pub fn new(s3: Arc<moatless_s3::S3Storage>, bucket: String, prefix: String) -> Self {
        Self {
            s3,
            bucket,
            prefix,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The task's file set and the time it was stored, from cache when fresh.
    async fn nodes(
        &self,
        task_id: &str,
    ) -> Result<(Arc<Vec<SnapshotNode>>, DateTime<Utc>), SnapshotError> {
        {
            let cache = self.cache.lock().await;
            if let Some(hit) = cache.get(task_id) {
                if hit.fetched.elapsed() < MANIFEST_TTL {
                    return Ok((Arc::clone(&hit.nodes), hit.synced_at));
                }
            }
        }

        let key = manifest_key(&self.prefix, task_id);
        let synced_at =
            match moatless_s3::head_object_modified_at(&self.s3.client, &self.bucket, &key).await {
                Ok(Some(at)) => at,
                Ok(None) => return Err(SnapshotError::NoSnapshot),
                Err(e) => return Err(SnapshotError::Unavailable(e.to_string())),
            };

        let bytes = match moatless_s3::get_object_opt(&self.s3.client, &self.bucket, &key).await {
            Ok(Some(bytes)) => bytes,
            // The HEAD above saw it, so a miss here is a delete racing this read
            // rather than a task without a mirror.
            Ok(None) => return Err(SnapshotError::NoSnapshot),
            Err(e) => return Err(SnapshotError::Unavailable(e.to_string())),
        };

        let manifest = Manifest::from_gz_json(&bytes)
            .map_err(|e| SnapshotError::Unavailable(format!("manifest is unreadable: {e}")))?;
        if manifest.version != MANIFEST_VERSION {
            warn!(
                task_id = %task_id,
                version = manifest.version,
                "Stored manifest is a version this build cannot read"
            );
            return Err(SnapshotError::NoSnapshot);
        }

        debug!(task_id = %task_id, files = manifest.entries.len(), "Loaded a workspace snapshot manifest");
        let nodes: Arc<Vec<SnapshotNode>> = Arc::new(
            manifest
                .entries
                .into_iter()
                .map(|entry| SnapshotNode {
                    path: entry.path,
                    size: entry.size,
                    symlink: entry.symlink,
                })
                .collect(),
        );
        let mut cache = self.cache.lock().await;
        cache.retain(|_, held| held.fetched.elapsed() < MANIFEST_TTL);
        if cache.len() >= MANIFEST_CACHE_CAP {
            if let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, held)| held.fetched)
                .map(|(id, _)| id.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(
            task_id.to_string(),
            Cached {
                nodes: Arc::clone(&nodes),
                synced_at,
                fetched: Instant::now(),
            },
        );
        Ok((nodes, synced_at))
    }
}

#[async_trait]
impl WorkspaceSnapshotSource for S3WorkspaceSnapshots {
    async fn read(&self, task_id: &str, path: &str) -> Result<SnapshotFile, SnapshotError> {
        let (nodes, synced_at) = self.nodes(task_id).await?;
        let stored =
            resolve(&nodes, path.trim_start_matches('/')).ok_or(SnapshotError::NotFound)?;

        // The same rule the sandbox applied when it named the object. A path
        // that fails it cannot have been stored, so this is a miss and not a
        // refusal.
        if !is_safe_workspace_path(stored) {
            return Err(SnapshotError::NotFound);
        }

        let key = workspace_object_key(&self.prefix, task_id, stored);
        match moatless_s3::get_object_opt(&self.s3.client, &self.bucket, &key).await {
            Ok(Some(content)) => Ok(SnapshotFile { content, synced_at }),
            Ok(None) => Err(SnapshotError::NotFound),
            Err(e) => Err(SnapshotError::Unavailable(e.to_string())),
        }
    }

    async fn list(
        &self,
        task_id: &str,
        path: &str,
        recursive: bool,
    ) -> Result<SnapshotListing, SnapshotError> {
        let (nodes, synced_at) = self.nodes(task_id).await?;
        Ok(SnapshotListing {
            entries: listing_under(&nodes, &dir_prefix(path), recursive),
            synced_at,
        })
    }

    async fn search(
        &self,
        task_id: &str,
        query: &str,
        case_sensitive: bool,
        limit: usize,
    ) -> Result<SnapshotListing, SnapshotError> {
        let (nodes, synced_at) = self.nodes(task_id).await?;
        Ok(SnapshotListing {
            entries: search_under(&nodes, query, case_sensitive, limit),
            synced_at,
        })
    }
}
