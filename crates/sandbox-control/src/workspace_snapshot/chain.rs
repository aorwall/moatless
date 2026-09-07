//! Answering from the first backing that holds a snapshot.
//!
//! A deployment can hold two generations at once: tasks whose sandbox wrote a
//! `restic` repository, and older ones that only ever wrote the file mirror.
//! The chain is what lets one reader serve both, so switching a sandbox to the
//! new format needs no migration of what is already stored.

use async_trait::async_trait;
use moatless_ports::sandbox::{
    SnapshotError, SnapshotFile, SnapshotListing, WorkspaceSnapshotSource,
};
use tracing::warn;

/// [`WorkspaceSnapshotSource`] over an ordered list of backings.
pub struct WorkspaceSnapshotChain {
    backings: Vec<(&'static str, Box<dyn WorkspaceSnapshotSource>)>,
}

impl WorkspaceSnapshotChain {
    /// Try each backing in order. Name each one: a log line then says which
    /// backing failed, which is what tells "this task has no repository" from
    /// "the repository would not open".
    pub fn new(backings: Vec<(&'static str, Box<dyn WorkspaceSnapshotSource>)>) -> Self {
        Self { backings }
    }

    /// Whether one backing's failure sends the task to the next backing.
    ///
    /// Only [`SnapshotError::NoSnapshot`] moves on. [`SnapshotError::NotFound`]
    /// is an answer — the backing holds this task and not that path — and
    /// [`SnapshotError::Unavailable`] is an outage every backing in this chain
    /// shares, so trying again would only trade one error for another.
    fn keep_looking(task_id: &str, backing: &str, error: &SnapshotError) -> bool {
        match error {
            SnapshotError::NoSnapshot => true,
            SnapshotError::Unavailable(reason) => {
                warn!(task_id = %task_id, backing, reason = %reason, "Workspace snapshot backing is unreadable");
                false
            }
            SnapshotError::NotFound => false,
        }
    }
}

#[async_trait]
impl WorkspaceSnapshotSource for WorkspaceSnapshotChain {
    async fn read(&self, task_id: &str, path: &str) -> Result<SnapshotFile, SnapshotError> {
        for (name, backing) in &self.backings {
            match backing.read(task_id, path).await {
                Ok(found) => return Ok(found),
                Err(err) if Self::keep_looking(task_id, name, &err) => {}
                Err(err) => return Err(err),
            }
        }
        Err(SnapshotError::NoSnapshot)
    }

    async fn list(
        &self,
        task_id: &str,
        path: &str,
        recursive: bool,
    ) -> Result<SnapshotListing, SnapshotError> {
        for (name, backing) in &self.backings {
            match backing.list(task_id, path, recursive).await {
                Ok(found) => return Ok(found),
                Err(err) if Self::keep_looking(task_id, name, &err) => {}
                Err(err) => return Err(err),
            }
        }
        Err(SnapshotError::NoSnapshot)
    }

    async fn search(
        &self,
        task_id: &str,
        query: &str,
        case_sensitive: bool,
        limit: usize,
    ) -> Result<SnapshotListing, SnapshotError> {
        for (name, backing) in &self.backings {
            match backing.search(task_id, query, case_sensitive, limit).await {
                Ok(found) => return Ok(found),
                Err(err) if Self::keep_looking(task_id, name, &err) => {}
                Err(err) => return Err(err),
            }
        }
        Err(SnapshotError::NoSnapshot)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use moatless_ports::sandbox::MockWorkspaceSnapshotSource;

    use super::*;

    fn listing() -> SnapshotListing {
        SnapshotListing {
            entries: Vec::new(),
            synced_at: Utc::now(),
        }
    }

    fn chain(
        first: MockWorkspaceSnapshotSource,
        second: MockWorkspaceSnapshotSource,
    ) -> WorkspaceSnapshotChain {
        WorkspaceSnapshotChain::new(vec![
            ("first", Box::new(first)),
            ("second", Box::new(second)),
        ])
    }

    #[tokio::test]
    async fn a_backing_without_a_snapshot_hands_the_task_to_the_next_one() {
        let mut first = MockWorkspaceSnapshotSource::new();
        first
            .expect_list()
            .returning(|_, _, _| Err(SnapshotError::NoSnapshot));
        let mut second = MockWorkspaceSnapshotSource::new();
        second
            .expect_list()
            .times(1)
            .returning(|_, _, _| Ok(listing()));

        assert!(chain(first, second).list("task", ".", false).await.is_ok());
    }

    /// A path the first backing does not hold is an answer, not a reason to ask
    /// the second: both backings describe the same workspace.
    #[tokio::test]
    async fn a_missing_path_does_not_reach_the_next_backing() {
        let mut first = MockWorkspaceSnapshotSource::new();
        first
            .expect_read()
            .returning(|_, _| Err(SnapshotError::NotFound));
        let mut second = MockWorkspaceSnapshotSource::new();
        second.expect_read().never();

        assert!(matches!(
            chain(first, second).read("task", "a.txt").await,
            Err(SnapshotError::NotFound)
        ));
    }

    /// Every backing here is the same bucket, so an outage is the answer rather
    /// than a reason to try again and report a different error.
    #[tokio::test]
    async fn an_outage_stops_the_chain_instead_of_reading_as_no_snapshot() {
        let mut first = MockWorkspaceSnapshotSource::new();
        first
            .expect_search()
            .returning(|_, _, _, _| Err(SnapshotError::Unavailable("s3 timed out".into())));
        let mut second = MockWorkspaceSnapshotSource::new();
        second.expect_search().never();

        assert!(matches!(
            chain(first, second).search("task", "q", false, 0).await,
            Err(SnapshotError::Unavailable(_))
        ));
    }

    #[tokio::test]
    async fn a_task_no_backing_holds_reads_as_no_snapshot() {
        let mut first = MockWorkspaceSnapshotSource::new();
        first
            .expect_list()
            .returning(|_, _, _| Err(SnapshotError::NoSnapshot));
        let mut second = MockWorkspaceSnapshotSource::new();
        second
            .expect_list()
            .returning(|_, _, _| Err(SnapshotError::NoSnapshot));

        assert!(matches!(
            chain(first, second).list("task", ".", false).await,
            Err(SnapshotError::NoSnapshot)
        ));
    }
}
