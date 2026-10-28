//! The record tables, as a host, the collection routes and the artifact
//! routes reach them.

use std::sync::Arc;

use moatless_workspace_ports::{RepositoryReader, WorkspaceResolver};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::dao::RecordDao;
use super::keys::MAX_RECORDS;
use super::watch::RecordWatch;
use crate::scope_reach::ScopeOwners;

/// Record collections, their records, and this replica's watch on them.
///
/// Scope resolution and the access check stay in
/// [`crate::collection::service`] and, for a page, [`crate::artifact::records`].
pub struct Records {
    pub(crate) store: Arc<dyn RecordDao>,
    repositories: Arc<dyn RepositoryReader>,
    workspaces: Arc<dyn WorkspaceResolver>,
    watch: RecordWatch,
}

impl Records {
    pub(crate) fn new(
        store: Arc<dyn RecordDao>,
        repositories: Arc<dyn RepositoryReader>,
        workspaces: Arc<dyn WorkspaceResolver>,
    ) -> Self {
        Self {
            store,
            repositories,
            workspaces,
            watch: RecordWatch::default(),
        }
    }

    /// Whose Workspace or repository a scoped collection belongs to.
    pub(crate) fn owners(&self) -> ScopeOwners<'_> {
        ScopeOwners {
            workspaces: self.workspaces.as_ref(),
            repositories: self.repositories.as_ref(),
        }
    }

    /// Whether a save under `key` fits: the collection holds fewer than
    /// [`MAX_RECORDS`], or already holds `key`.
    pub(crate) async fn has_room(
        &self,
        collection_id: &str,
        key: &str,
    ) -> Result<bool, sqlx::Error> {
        let (count, key_exists) = self.store.count_records(collection_id, key).await?;
        Ok(count < MAX_RECORDS || key_exists)
    }

    /// The collection's `record_seq`; see [`RecordDao::record_heads`].
    pub(crate) async fn head(&self, collection_id: &str) -> Result<i64, sqlx::Error> {
        let heads = self
            .store
            .record_heads(&[collection_id.to_string()])
            .await?;
        Ok(heads.first().map_or(0, |(_, seq)| *seq))
    }

    /// This replica's watch on the `record_seq` of the collections pages here
    /// are subscribed to.
    pub(crate) fn watch(&self) -> &RecordWatch {
        &self.watch
    }

    /// Keep [`Self::watch`] current until `cancel`.
    pub fn spawn_watch(self: &Arc<Self>, cancel: CancellationToken) -> JoinHandle<()> {
        let records = Arc::clone(self);
        tokio::spawn(async move {
            records.watch.run(records.store.as_ref(), cancel).await;
        })
    }
}
