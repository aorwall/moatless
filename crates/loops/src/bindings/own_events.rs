//! The external events a Task made: one writer and one reader for every
//! Adapter, so an event a Task made never comes back to it.
//!
//! An outbound transport (a reply, a proxied push or API write, an Adapter
//! command) ends in [`OwnEvents::record`]; inbound routing asks
//! [`OwnEvents::made_by`]. The in-memory window covers a webhook that lands
//! before the durable row commits; the row covers everything after.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moatless_adapter_model::registry::Adapters;
use moatless_ports::task::{EventSubject, ExternalEvent, TranscriptAppend};
use moatless_types::store::StoreError;
use tracing::warn;

/// Above realistic webhook latency, below realistic external-id reuse.
const RECENT_TTL: Duration = Duration::from_mins(2);

/// What every Task made on every Adapter, read and written in one place.
pub struct OwnEvents {
    recent: Mutex<HashMap<String, (String, Instant)>>,
    transcript: Arc<dyn TranscriptAppend>,
    adapters: Option<Arc<Adapters>>,
}

impl OwnEvents {
    pub fn new(transcript: Arc<dyn TranscriptAppend>, adapters: Option<Arc<Adapters>>) -> Self {
        Self {
            recent: Mutex::new(HashMap::new()),
            transcript,
            adapters,
        }
    }

    /// Record that `task_id` made `event`. Idempotent: the durable row is
    /// keyed on the external id, so a retried or doubled record is one row.
    pub async fn record(&self, task_id: &str, event: ExternalEvent) -> Result<(), StoreError> {
        self.note(task_id, &event.external_id);
        self.transcript.record_external_event(task_id, &event).await
    }

    /// [`Self::record`] for a caller that cannot act on a failure: the
    /// in-memory window still holds, so only a webhook later than
    /// [`RECENT_TTL`] could echo.
    pub async fn record_best_effort(&self, task_id: &str, event: ExternalEvent) -> bool {
        let external_id = event.external_id.clone();
        match self.record(task_id, event).await {
            Ok(()) => true,
            Err(error) => {
                warn!(task_id, external_id, %error, "Failed to record a Task's own external event");
                false
            }
        }
    }

    /// Note an event whose durable row the caller writes itself, in the same
    /// transaction as its transcript row (`TranscriptEntry::external`).
    pub fn note(&self, task_id: &str, external_id: &str) {
        if let Ok(mut recent) = self.recent.lock() {
            recent.insert(
                external_id.to_string(),
                (task_id.to_string(), Instant::now()),
            );
        }
    }

    /// The Task that made `external_id` on `subject`, when one can be named.
    ///
    /// Scoped to the subject: a provider numbering events per subject reuses
    /// one id across subjects, so a Task holding it elsewhere didn't make this.
    pub async fn made_by(
        &self,
        external_id: &str,
        subject: EventSubject<'_>,
    ) -> Result<Option<String>, StoreError> {
        if !self.names_producer(subject.adapter, external_id) {
            return Ok(None);
        }
        let candidates = self.aliases(subject.adapter, external_id);
        if let Some(task_id) = self.recent_maker(&candidates) {
            return Ok(Some(task_id));
        }
        self.transcript
            .find_author_on_subject(&candidates, subject)
            .await
    }

    /// The Task that made the comment `external_id` names, wherever it was
    /// written. Unscoped: correct only for a per-comment id the provider
    /// allocates once globally.
    pub async fn comment_made_by(
        &self,
        adapter: &str,
        external_id: &str,
    ) -> Result<Option<String>, StoreError> {
        let candidates = self.aliases(adapter, external_id);
        if let Some(task_id) = self.recent_maker(&candidates) {
            return Ok(Some(task_id));
        }
        self.transcript.find_comment_author(&candidates).await
    }

    /// Every external event id `task_id` made.
    pub async fn made(&self, task_id: &str) -> Result<Vec<String>, StoreError> {
        self.transcript.external_event_ids(task_id).await
    }

    /// Whether `task_id` made `external_id` within the in-memory window.
    pub fn made_recently(&self, task_id: &str, external_id: &str) -> bool {
        self.recent_maker(&[external_id.to_string()])
            .is_some_and(|maker| maker == task_id)
    }

    /// Every id `external_id` is also recorded under, by its Adapter's codec.
    pub fn aliases(&self, adapter: &str, external_id: &str) -> Vec<String> {
        self.adapters
            .as_ref()
            .and_then(|adapters| adapters.inbound_presentation_by_name(adapter))
            .map_or_else(
                || vec![external_id.to_string()],
                |presentation| presentation.external_id_aliases(external_id),
            )
    }

    fn names_producer(&self, adapter: &str, external_id: &str) -> bool {
        self.adapters
            .as_ref()
            .and_then(|adapters| adapters.inbound_presentation_by_name(adapter))
            .is_none_or(|presentation| presentation.names_producer(external_id))
    }

    fn recent_maker(&self, candidates: &[String]) -> Option<String> {
        let mut recent = self.recent.lock().ok()?;
        let now = Instant::now();
        recent.retain(|_, (_, at)| now.duration_since(*at) < RECENT_TTL);
        candidates
            .iter()
            .find_map(|id| recent.get(id).map(|(task_id, _)| task_id.clone()))
    }
}

/// The insert-time check `send.rs` runs on every turn carrying an event id.
impl moatless_tasks::message::send::SelfEchoGuard for OwnEvents {
    fn sent_recently(&self, task_id: &str, ext_id: &str) -> bool {
        self.made_recently(task_id, ext_id)
    }

    fn candidate_ids(&self, adapter: Option<&str>, ext_id: &str) -> Vec<String> {
        adapter.map_or_else(
            || vec![ext_id.to_string()],
            |adapter| self.aliases(adapter, ext_id),
        )
    }
}

#[cfg(test)]
mod tests {
    use moatless_ports::task::MockTranscriptAppend;

    use super::*;

    fn subject() -> EventSubject<'static> {
        EventSubject {
            adapter: "github_pr",
            source_name: "acme/app",
            subject_key: "acme/app#7",
        }
    }

    fn event(external_id: &str) -> ExternalEvent {
        ExternalEvent {
            external_id: external_id.to_string(),
            provider: "github_pr".to_string(),
            source_name: Some("acme/app".to_string()),
        }
    }

    #[tokio::test]
    async fn a_recorded_event_is_made_by_its_task_before_the_row_is_read() {
        let mut transcript = MockTranscriptAppend::new();
        transcript
            .expect_record_external_event()
            .times(1)
            .returning(|_, _| Ok(()));
        transcript.expect_find_author_on_subject().never();
        let own = OwnEvents::new(Arc::new(transcript), None);

        own.record("task-1", event("github:push:acme/app:abc"))
            .await
            .unwrap();

        assert_eq!(
            own.made_by("github:push:acme/app:abc", subject())
                .await
                .unwrap()
                .as_deref(),
            Some("task-1")
        );
        assert!(own.made_recently("task-1", "github:push:acme/app:abc"));
        assert!(!own.made_recently("task-2", "github:push:acme/app:abc"));
    }

    #[tokio::test]
    async fn an_event_outside_the_window_is_read_from_the_durable_row() {
        let mut transcript = MockTranscriptAppend::new();
        transcript
            .expect_find_author_on_subject()
            .times(1)
            .returning(|_, _| Ok(Some("task-9".to_string())));
        let own = OwnEvents::new(Arc::new(transcript), None);

        assert_eq!(
            own.made_by("github:ic:42", subject())
                .await
                .unwrap()
                .as_deref(),
            Some("task-9")
        );
    }

    #[tokio::test]
    async fn an_expired_note_names_no_task() {
        let mut transcript = MockTranscriptAppend::new();
        transcript
            .expect_find_author_on_subject()
            .returning(|_, _| Ok(None));
        let own = OwnEvents::new(Arc::new(transcript), None);
        own.note("task-1", "github:ic:42");
        {
            let mut recent = own.recent.lock().unwrap();
            let stale = Instant::now()
                .checked_sub(RECENT_TTL + Duration::from_secs(1))
                .expect("the process has been up longer than the TTL");
            recent.insert("github:ic:42".to_string(), ("task-1".to_string(), stale));
        }

        assert!(
            own.made_by("github:ic:42", subject())
                .await
                .unwrap()
                .is_none()
        );
        assert!(!own.made_recently("task-1", "github:ic:42"));
    }
}
