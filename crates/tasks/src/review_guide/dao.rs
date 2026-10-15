//! Persistence for guided reviews: one stored guide and at most one run per
//! pull request.

use async_trait::async_trait;

use super::models::{
    PullRequestKey, RUN_FAILED, RUN_RUNNING, ReviewGuideRun, RunClaim, StoredReviewGuide,
};
use moatless_core::db::DbPool;

const RUN_COLUMNS: &str = "host, repository, number, run_id, head_sha, base_sha, task_id, status, \
                           detail, started_at, failed_at";

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ReviewGuideDao: Send + Sync {
    async fn guide(&self, key: &PullRequestKey) -> Result<Option<StoredReviewGuide>, sqlx::Error>;

    async fn run(&self, key: &PullRequestKey) -> Result<Option<ReviewGuideRun>, sqlx::Error>;

    /// The running run a Task was started to write, if it still owns one.
    async fn running_run_for_task(
        &self,
        task_id: &str,
    ) -> Result<Option<ReviewGuideRun>, sqlx::Error>;

    /// Start a run for `head_sha`, or join the one already running for it.
    ///
    /// A running run for another head, or a failed one, is replaced: a newer
    /// claim owns the pull request, and the older run's guide is dropped when
    /// it is submitted.
    async fn claim_run(
        &self,
        key: &PullRequestKey,
        run_id: &str,
        head_sha: &str,
        base_sha: &str,
    ) -> Result<RunClaim, sqlx::Error>;

    /// Record the Task writing `run_id`. False when a newer claim replaced it
    /// or it was failed.
    async fn attach_task(&self, run_id: &str, task_id: &str) -> Result<bool, sqlx::Error>;

    /// Mark `run_id` failed with `detail`, if it is still running.
    async fn fail_run(&self, run_id: &str, detail: &str) -> Result<(), sqlx::Error>;

    /// Store `guide` for the pull request `run_id` belongs to and end the run,
    /// in one transaction. False when the run no longer owns the pull request,
    /// in which case nothing is written.
    async fn complete_run(
        &self,
        run_id: &str,
        guide: &StoredReviewGuide,
    ) -> Result<bool, sqlx::Error>;
}

pub struct PgReviewGuideDao(pub DbPool);

#[async_trait]
impl ReviewGuideDao for PgReviewGuideDao {
    async fn guide(&self, key: &PullRequestKey) -> Result<Option<StoredReviewGuide>, sqlx::Error> {
        sqlx::query_as::<_, StoredReviewGuide>(
            r#"SELECT head_sha, guide, task_id, agent_profile, model, generated_at
               FROM pull_request_review_guides
               WHERE host = $1 AND repository = $2 AND number = $3"#,
        )
        .bind(&key.host)
        .bind(&key.repository)
        .bind(key.number)
        .fetch_optional(&self.0)
        .await
    }

    async fn run(&self, key: &PullRequestKey) -> Result<Option<ReviewGuideRun>, sqlx::Error> {
        sqlx::query_as::<_, ReviewGuideRun>(sqlx::AssertSqlSafe(format!(
            "SELECT {RUN_COLUMNS} FROM pull_request_review_guide_runs
             WHERE host = $1 AND repository = $2 AND number = $3"
        )))
        .bind(&key.host)
        .bind(&key.repository)
        .bind(key.number)
        .fetch_optional(&self.0)
        .await
    }

    async fn running_run_for_task(
        &self,
        task_id: &str,
    ) -> Result<Option<ReviewGuideRun>, sqlx::Error> {
        sqlx::query_as::<_, ReviewGuideRun>(sqlx::AssertSqlSafe(format!(
            "SELECT {RUN_COLUMNS} FROM pull_request_review_guide_runs
             WHERE task_id = $1 AND status = '{RUN_RUNNING}'"
        )))
        .bind(task_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn claim_run(
        &self,
        key: &PullRequestKey,
        run_id: &str,
        head_sha: &str,
        base_sha: &str,
    ) -> Result<RunClaim, sqlx::Error> {
        // The WHERE on the upsert leaves a running run for the same head
        // alone, so a second press joins it rather than starting another
        // Task; the row returned is then the existing one.
        let claimed = sqlx::query_as::<_, ReviewGuideRun>(sqlx::AssertSqlSafe(format!(
            "INSERT INTO pull_request_review_guide_runs
                 (host, repository, number, run_id, head_sha, base_sha, status)
             VALUES ($1, $2, $3, $4, $5, $6, '{RUN_RUNNING}')
             ON CONFLICT (host, repository, number) DO UPDATE
                 SET run_id = EXCLUDED.run_id,
                     head_sha = EXCLUDED.head_sha,
                     base_sha = EXCLUDED.base_sha,
                     task_id = NULL,
                     status = '{RUN_RUNNING}',
                     detail = NULL,
                     started_at = now(),
                     failed_at = NULL
                 WHERE pull_request_review_guide_runs.status <> '{RUN_RUNNING}'
                    OR pull_request_review_guide_runs.head_sha <> EXCLUDED.head_sha
             RETURNING {RUN_COLUMNS}"
        )))
        .bind(&key.host)
        .bind(&key.repository)
        .bind(key.number)
        .bind(run_id)
        .bind(head_sha)
        .bind(base_sha)
        .fetch_optional(&self.0)
        .await?;
        if let Some(run) = claimed {
            return Ok(RunClaim::Claimed(run));
        }
        // Nothing was written, so a running run for this head holds the row.
        // A failure between the two reads reports the row as it now stands.
        match self.run(key).await? {
            Some(run) => Ok(RunClaim::Joined(run)),
            None => Err(sqlx::Error::RowNotFound),
        }
    }

    async fn attach_task(&self, run_id: &str, task_id: &str) -> Result<bool, sqlx::Error> {
        let updated = sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE pull_request_review_guide_runs SET task_id = $2
              WHERE run_id = $1 AND status = '{RUN_RUNNING}'"
        )))
        .bind(run_id)
        .bind(task_id)
        .execute(&self.0)
        .await?;
        Ok(updated.rows_affected() > 0)
    }

    async fn fail_run(&self, run_id: &str, detail: &str) -> Result<(), sqlx::Error> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE pull_request_review_guide_runs
                SET status = '{RUN_FAILED}', detail = $2, failed_at = now()
              WHERE run_id = $1 AND status = '{RUN_RUNNING}'"
        )))
        .bind(run_id)
        .bind(detail)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn complete_run(
        &self,
        run_id: &str,
        guide: &StoredReviewGuide,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        // Deleting the run is the ownership check: a newer claim replaced
        // `run_id` on the row, so this matches nothing and the guide, written
        // for an older head, is dropped.
        let ended = sqlx::query_as::<_, (String, String, i64)>(sqlx::AssertSqlSafe(format!(
            "DELETE FROM pull_request_review_guide_runs
              WHERE run_id = $1 AND status = '{RUN_RUNNING}'
              RETURNING host, repository, number"
        )))
        .bind(run_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((host, repository, number)) = ended else {
            return Ok(false);
        };
        sqlx::query(
            r#"INSERT INTO pull_request_review_guides
                   (host, repository, number, head_sha, guide, task_id, agent_profile, model,
                    generated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               ON CONFLICT (host, repository, number) DO UPDATE
                   SET head_sha = EXCLUDED.head_sha,
                       guide = EXCLUDED.guide,
                       task_id = EXCLUDED.task_id,
                       agent_profile = EXCLUDED.agent_profile,
                       model = EXCLUDED.model,
                       generated_at = EXCLUDED.generated_at"#,
        )
        .bind(&host)
        .bind(&repository)
        .bind(number)
        .bind(&guide.head_sha)
        .bind(&guide.guide)
        .bind(&guide.task_id)
        .bind(&guide.agent_profile)
        .bind(&guide.model)
        .bind(guide.generated_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }
}
