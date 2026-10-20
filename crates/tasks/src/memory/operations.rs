//! The memory tables, as a host and the memory routes reach them.

use std::sync::Arc;

use moatless_workspace_ports::{RepositoryReader, WorkspaceResolver};

use super::dao::MemoryDao;

/// Memory entries, their history and their reads.
///
/// Scope resolution and the access check stay in [`crate::memory::service`].
pub struct Memories {
    pub(crate) store: Arc<dyn MemoryDao>,
    /// Whose repository a `repository` entry belongs to, for the write check.
    pub(crate) repositories: Arc<dyn RepositoryReader>,
    /// Whose Workspace a `workspace` entry belongs to, for the write check.
    pub(crate) workspaces: Arc<dyn WorkspaceResolver>,
}

impl Memories {
    pub(crate) fn new(
        store: Arc<dyn MemoryDao>,
        repositories: Arc<dyn RepositoryReader>,
        workspaces: Arc<dyn WorkspaceResolver>,
    ) -> Self {
        Self {
            store,
            repositories,
            workspaces,
        }
    }
}
