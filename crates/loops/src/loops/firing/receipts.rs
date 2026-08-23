//! Durable adapter-event deduplication.

use moatless_core::db::DbPool;

pub struct PgEventReceiptDao(DbPool);

#[async_trait::async_trait]
pub trait EventReceiptDao: Send + Sync {
    async fn record_once(
        &self,
        provider: &str,
        external_event_id: &str,
        subject_kind: &str,
        subject_key: &str,
        task_id: &str,
    ) -> Result<bool, sqlx::Error>;

    /// Claim the right to create the task for a subject.
    ///
    /// `record_once` keys on `task_id`, so it can only speak for a task that
    /// already exists. Creation needs the opposite: a decision taken before
    /// any task exists, and taken once across every replica.
    async fn claim_task_creation(
        &self,
        loop_id: &str,
        subject_kind: &str,
        subject_key: &str,
        adapter_kind: &str,
        external_event_id: Option<&str>,
    ) -> Result<bool, sqlx::Error>;

    /// Record which task won a claim, so a loser can find it.
    async fn record_claimed_task(
        &self,
        loop_id: &str,
        subject_kind: &str,
        subject_key: &str,
        task_id: &str,
    ) -> Result<(), sqlx::Error>;

    /// Release a claim whose task was never created.
    ///
    /// Without this, a creation that fails after claiming would wedge the
    /// subject: every later event would lose a claim held by nothing.
    async fn release_claim(
        &self,
        loop_id: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<(), sqlx::Error>;
}

impl PgEventReceiptDao {
    pub fn new(pool: DbPool) -> Self {
        Self(pool)
    }

    /// Insert an event receipt.
    ///
    /// Returns `true` when this is the first time the event has been seen for
    /// the task, `false` when a unique-key conflict proves it was delivered
    /// already.
    pub async fn record_once(
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

    /// Insert the creation claim for a subject.
    ///
    /// Returns `true` for the one caller that lands the row. The insert *is*
    /// the decision — no read precedes it, so two replicas racing cannot both
    /// come away believing they should create.
    pub async fn claim_task_creation(
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

    pub async fn record_claimed_task(
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

    pub async fn release_claim(
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

#[async_trait::async_trait]
impl EventReceiptDao for PgEventReceiptDao {
    async fn record_once(
        &self,
        provider: &str,
        external_event_id: &str,
        subject_kind: &str,
        subject_key: &str,
        task_id: &str,
    ) -> Result<bool, sqlx::Error> {
        PgEventReceiptDao::record_once(
            self,
            provider,
            external_event_id,
            subject_kind,
            subject_key,
            task_id,
        )
        .await
    }

    async fn claim_task_creation(
        &self,
        loop_id: &str,
        subject_kind: &str,
        subject_key: &str,
        adapter_kind: &str,
        external_event_id: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        PgEventReceiptDao::claim_task_creation(
            self,
            loop_id,
            subject_kind,
            subject_key,
            adapter_kind,
            external_event_id,
        )
        .await
    }

    async fn record_claimed_task(
        &self,
        loop_id: &str,
        subject_kind: &str,
        subject_key: &str,
        task_id: &str,
    ) -> Result<(), sqlx::Error> {
        PgEventReceiptDao::record_claimed_task(self, loop_id, subject_kind, subject_key, task_id)
            .await
    }

    async fn release_claim(
        &self,
        loop_id: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<(), sqlx::Error> {
        PgEventReceiptDao::release_claim(self, loop_id, subject_kind, subject_key).await
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use sqlx::PgPool;

    const LOOP_ID: &str = "tr_claim_test";
    const KIND: &str = "slack_thread";
    const SUBJECT: &str = "T1:C1:1787469709.912279";

    fn dao(pool: PgPool) -> PgEventReceiptDao {
        PgEventReceiptDao::new(pool)
    }

    /// Verification: the incident this table exists for. One Slack message
    /// reached the backend as two events ~300ms apart, on two replicas, and
    /// both created a task because each had read an empty binding list first.
    /// The claim removes the read: the insert is the decision, so concurrency
    /// cannot produce two winners.
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
}
