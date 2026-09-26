//! Durable adapter-event deduplication.

use moatless_core::db::DbPool;

pub(crate) struct EventReceipts(DbPool);

impl EventReceipts {
    pub(crate) fn new(pool: DbPool) -> Self {
        Self(pool)
    }

    /// Insert an event receipt.
    ///
    /// Returns `true` the first time the event is seen for the task, `false`
    /// when a unique-key conflict proves it was already delivered.
    pub(crate) async fn record_once(
        &self,
        provider: &str,
        external_event_id: &str,
        subject_kind: &str,
        subject_key: &str,
        task_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"INSERT INTO adapter_event_receipts
               (adapter_kind, external_event_id, subject_kind, subject_key, task_id)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (adapter_kind, external_event_id, task_id) DO NOTHING"#,
        )
        .bind(provider)
        .bind(external_event_id)
        .bind(subject_kind)
        .bind(subject_key)
        .bind(task_id)
        .execute(&self.0)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    /// Claim the right to create the task for a subject.
    ///
    /// Unlike [`Self::record_once`], which keys on an existing `task_id`, the
    /// insert here *is* the decision — no read precedes it, so racing replicas
    /// cannot both come away believing they should create; the winner gets
    /// `true`.
    pub(crate) async fn claim_task_creation(
        &self,
        loop_id: &str,
        subject_kind: &str,
        subject_key: &str,
        adapter_kind: &str,
        external_event_id: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"INSERT INTO adapter_task_creation_claims
               (loop_id, subject_kind, subject_key, adapter_kind, external_event_id)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (loop_id, subject_kind, subject_key) DO NOTHING"#,
        )
        .bind(loop_id)
        .bind(subject_kind)
        .bind(subject_key)
        .bind(adapter_kind)
        .bind(external_event_id)
        .execute(&self.0)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    /// Record which task won a claim, so a loser can find it.
    pub(crate) async fn record_claimed_task(
        &self,
        loop_id: &str,
        subject_kind: &str,
        subject_key: &str,
        task_id: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE adapter_task_creation_claims SET task_id = $4
               WHERE loop_id = $1 AND subject_kind = $2 AND subject_key = $3"#,
        )
        .bind(loop_id)
        .bind(subject_kind)
        .bind(subject_key)
        .bind(task_id)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    /// The task a subject's claim produced; `None` while its creation is in flight.
    pub(crate) async fn claimed_task(
        &self,
        loop_id: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        let task_id: Option<Option<String>> = sqlx::query_scalar(
            r#"SELECT task_id FROM adapter_task_creation_claims
               WHERE loop_id = $1 AND subject_kind = $2 AND subject_key = $3"#,
        )
        .bind(loop_id)
        .bind(subject_kind)
        .bind(subject_key)
        .fetch_optional(&self.0)
        .await?;
        Ok(task_id.flatten())
    }

    /// Take over a claim still pointing at `stale_task_id`, as a new creation.
    ///
    /// Conditioned on that task id, so of several racing callers exactly one
    /// wins: the first update clears it, and the rest match no row.
    pub(crate) async fn take_over_claim(
        &self,
        loop_id: &str,
        subject_kind: &str,
        subject_key: &str,
        stale_task_id: &str,
        adapter_kind: &str,
        external_event_id: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"UPDATE adapter_task_creation_claims
               SET task_id = NULL, adapter_kind = $5, external_event_id = $6, claimed_at = now()
               WHERE loop_id = $1 AND subject_kind = $2 AND subject_key = $3
                 AND task_id = $4"#,
        )
        .bind(loop_id)
        .bind(subject_kind)
        .bind(subject_key)
        .bind(stale_task_id)
        .bind(adapter_kind)
        .bind(external_event_id)
        .execute(&self.0)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Release a claim whose task was never created.
    ///
    /// Without this, a failed creation would wedge the subject: every later
    /// event would lose a claim held by nothing.
    pub(crate) async fn release_claim(
        &self,
        loop_id: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"DELETE FROM adapter_task_creation_claims
               WHERE loop_id = $1 AND subject_kind = $2 AND subject_key = $3
                 AND task_id IS NULL"#,
        )
        .bind(loop_id)
        .bind(subject_kind)
        .bind(subject_key)
        .execute(&self.0)
        .await?;
        Ok(())
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use sqlx::PgPool;

    const LOOP_ID: &str = "tr_claim_test";
    const KIND: &str = "slack_thread";
    const SUBJECT: &str = "T1:C1:1787469709.912279";

    fn dao(pool: PgPool) -> EventReceipts {
        EventReceipts::new(pool)
    }

    /// Verification: the incident this table exists for.
    ///
    /// Two replicas each read an empty binding list and both created a task;
    /// the claim removes the read, so racing cannot produce two winners.
    #[sqlx::test]
    async fn only_one_of_two_racing_claims_wins(pool: PgPool) {
        let a = dao(pool.clone());
        let b = dao(pool.clone());

        let (first, second) = tokio::join!(
            a.claim_task_creation(LOOP_ID, KIND, SUBJECT, "slack", Some("slack:ts:1")),
            b.claim_task_creation(LOOP_ID, KIND, SUBJECT, "slack", Some("slack:ts:1")),
        );

        let won = [first.unwrap(), second.unwrap()];
        assert_eq!(
            won.iter().filter(|w| **w).count(),
            1,
            "exactly one delivery may create the task for a subject"
        );
    }

    /// Two events carrying *different* ids still collapse: the claim is keyed
    /// on the subject, not the event. Two quick messages opening the same
    /// thread are the same race as one message arriving twice.
    #[sqlx::test]
    async fn a_second_event_for_the_same_subject_loses(pool: PgPool) {
        let dao = dao(pool);
        assert!(
            dao.claim_task_creation(LOOP_ID, KIND, SUBJECT, "slack", Some("slack:ts:1"))
                .await
                .unwrap()
        );
        assert!(
            !dao.claim_task_creation(LOOP_ID, KIND, SUBJECT, "slack", Some("slack:ts:2"))
                .await
                .unwrap(),
            "a different event on a claimed subject must not create a second task"
        );
    }

    /// Subjects are independent. A busy channel must not serialise behind one
    /// thread's claim.
    #[sqlx::test]
    async fn a_different_subject_claims_freely(pool: PgPool) {
        let dao = dao(pool);
        assert!(
            dao.claim_task_creation(LOOP_ID, KIND, SUBJECT, "slack", None)
                .await
                .unwrap()
        );
        assert!(
            dao.claim_task_creation(LOOP_ID, KIND, "T1:C1:other", "slack", None)
                .await
                .unwrap()
        );
    }

    /// A claim whose task never materialised is handed back, or the subject is
    /// wedged: every later event would lose to a claim held by nothing and
    /// retry until its budget ran out.
    #[sqlx::test]
    async fn releasing_an_unfulfilled_claim_reopens_the_subject(pool: PgPool) {
        let dao = dao(pool);
        assert!(
            dao.claim_task_creation(LOOP_ID, KIND, SUBJECT, "slack", None)
                .await
                .unwrap()
        );
        dao.release_claim(LOOP_ID, KIND, SUBJECT).await.unwrap();
        assert!(
            dao.claim_task_creation(LOOP_ID, KIND, SUBJECT, "slack", None)
                .await
                .unwrap(),
            "the next event may create the task the failed one did not"
        );
    }

    /// Release is only for a claim that produced nothing. Once the winner has
    /// recorded its task, a later release must not reopen the subject — that
    /// would let a redelivery create a second task for a live thread.
    #[sqlx::test]
    async fn releasing_a_fulfilled_claim_does_nothing(pool: PgPool) {
        let dao = dao(pool);
        dao.claim_task_creation(LOOP_ID, KIND, SUBJECT, "slack", None)
            .await
            .unwrap();
        dao.record_claimed_task(LOOP_ID, KIND, SUBJECT, "task-1")
            .await
            .unwrap();

        dao.release_claim(LOOP_ID, KIND, SUBJECT).await.unwrap();

        assert!(
            !dao.claim_task_creation(LOOP_ID, KIND, SUBJECT, "slack", None)
                .await
                .unwrap(),
            "a subject whose task exists stays claimed"
        );
    }

    /// A claim outlives its task. Once that task closes, the next event for the
    /// subject takes the claim over, and only one of two racing events does.
    #[sqlx::test]
    async fn a_claim_whose_task_is_gone_is_taken_over_once(pool: PgPool) {
        let dao = dao(pool);
        assert!(
            dao.claim_task_creation(LOOP_ID, KIND, SUBJECT, "slack", None)
                .await
                .unwrap()
        );
        assert_eq!(
            dao.claimed_task(LOOP_ID, KIND, SUBJECT).await.unwrap(),
            None
        );
        dao.record_claimed_task(LOOP_ID, KIND, SUBJECT, "task-1")
            .await
            .unwrap();
        assert_eq!(
            dao.claimed_task(LOOP_ID, KIND, SUBJECT)
                .await
                .unwrap()
                .as_deref(),
            Some("task-1")
        );

        let (first, second) = tokio::join!(
            dao.take_over_claim(
                LOOP_ID,
                KIND,
                SUBJECT,
                "task-1",
                "slack",
                Some("slack:ts:2")
            ),
            dao.take_over_claim(
                LOOP_ID,
                KIND,
                SUBJECT,
                "task-1",
                "slack",
                Some("slack:ts:3")
            ),
        );
        assert_eq!(
            [first.unwrap(), second.unwrap()]
                .iter()
                .filter(|w| **w)
                .count(),
            1,
            "exactly one event may take over a claim"
        );
        assert_eq!(
            dao.claimed_task(LOOP_ID, KIND, SUBJECT).await.unwrap(),
            None,
            "a taken-over claim is in flight again"
        );
    }
}
