//! The guided-review tables, as a host and this module's handlers reach them.

use std::sync::Arc;

use super::dao::ReviewGuideDao;
use super::models::{PullRequestKey, ReviewGuideRun, RunClaim, StoredReviewGuide};

/// The guided-review tables. The backend holds this to read a guide and to
/// start a run; submission goes through [`crate::review_guide::service`].
pub struct ReviewGuides {
    store: Arc<dyn ReviewGuideDao>,
}

impl ReviewGuides {
    pub(crate) fn new(store: Arc<dyn ReviewGuideDao>) -> Self {
        Self { store }
    }

    /// See [`ReviewGuideDao::guide`].
    pub async fn guide(
        &self,
        key: &PullRequestKey,
    ) -> Result<Option<StoredReviewGuide>, sqlx::Error> {
        self.store.guide(key).await
    }

    /// See [`ReviewGuideDao::run`].
    pub async fn run(&self, key: &PullRequestKey) -> Result<Option<ReviewGuideRun>, sqlx::Error> {
        self.store.run(key).await
    }

    /// See [`ReviewGuideDao::claim_run`].
    pub async fn claim_run(
        &self,
        key: &PullRequestKey,
        head_sha: &str,
        base_sha: &str,
    ) -> Result<RunClaim, sqlx::Error> {
        let run_id = uuid::Uuid::new_v4().to_string();
        self.store.claim_run(key, &run_id, head_sha, base_sha).await
    }

    /// See [`ReviewGuideDao::attach_task`].
    pub async fn attach_task(&self, run_id: &str, task_id: &str) -> Result<bool, sqlx::Error> {
        self.store.attach_task(run_id, task_id).await
    }

    /// See [`ReviewGuideDao::fail_run`].
    pub async fn fail_run(&self, run_id: &str, detail: &str) -> Result<(), sqlx::Error> {
        self.store.fail_run(run_id, detail).await
    }

    pub(crate) async fn running_run_for_task(
        &self,
        task_id: &str,
    ) -> Result<Option<ReviewGuideRun>, sqlx::Error> {
        self.store.running_run_for_task(task_id).await
    }

    pub(crate) async fn complete_run(
        &self,
        run_id: &str,
        guide: &StoredReviewGuide,
    ) -> Result<bool, sqlx::Error> {
        self.store.complete_run(run_id, guide).await
    }
}
