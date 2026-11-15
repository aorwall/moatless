//! Storing the files an inbound message names, before the message is routed.

use std::fmt::Write as _;
use std::sync::Arc;

use moatless_adapter_model::capabilities::AdapterInboundFiles;
use moatless_core::file_storage::{Files, NewUpload};
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::{AdapterConnection, InboundAttachment, InboundMessage};

/// The largest file stored from an inbound message, matching what a T3 turn
/// may attach.
const MAX_ATTACHMENT_BYTES: u64 = 50 * 1024 * 1024;

/// How long one delivery may spend on its files, kept well under
/// `AdapterDeliveryPolicy::execution_timeout` (2 min) so routing still runs: a
/// download that times out is not stored, and every retry would wait for it
/// again, past the deadline, until the delivery fails with its text.
const STORE_BUDGET: std::time::Duration = std::time::Duration::from_secs(75);

/// Fetches an Adapter's inbound files and stores each as an upload.
pub struct InboundFiles {
    fetcher: Arc<dyn AdapterInboundFiles>,
    files: Arc<Files>,
}

impl InboundFiles {
    pub fn new(fetcher: Arc<dyn AdapterInboundFiles>, files: Arc<Files>) -> Self {
        Self { fetcher, files }
    }

    /// Move each of `message.attachments` into `message.file_ids`, appending a
    /// note to `message.text` for every one that could not be stored.
    ///
    /// Never fails the delivery, and returns within [`STORE_BUDGET`]: a retry
    /// would refetch every file, and the last one dropped would take the
    /// message's text with it, rather than losing only the file.
    pub(crate) async fn store(&self, connection: &AdapterConnection, message: &mut InboundMessage) {
        let deadline = tokio::time::Instant::now() + STORE_BUDGET;
        self.store_until(connection, message, deadline).await;
    }

    async fn store_until(
        &self,
        connection: &AdapterConnection,
        message: &mut InboundMessage,
        deadline: tokio::time::Instant,
    ) {
        let out_of_time = || "the delivery ran out of time to fetch it".to_string();
        for attachment in std::mem::take(&mut message.attachments) {
            let file_id = upload_id(connection, message, &attachment);
            let outcome = if tokio::time::Instant::now() >= deadline {
                Err(out_of_time())
            } else {
                tokio::time::timeout_at(
                    deadline,
                    self.store_one(connection, message, &attachment, &file_id),
                )
                .await
                .unwrap_or_else(|_| Err(out_of_time()))
            };
            match outcome {
                Ok(()) => message.file_ids.push(file_id),
                Err(reason) => {
                    tracing::warn!(
                        connection_id = %connection.id,
                        attachment = %attachment.external_id,
                        %reason,
                        "Inbound attachment not stored"
                    );
                    if !message.text.is_empty() {
                        message.text.push_str("\n\n");
                    }
                    let _ = write!(
                        message.text,
                        "[Attachment `{}` was not delivered: {reason}]",
                        attachment.filename
                    );
                }
            }
        }
    }

    /// Store one attachment under `file_id`, or say why it was not, in words
    /// the agent reading the message can act on.
    async fn store_one(
        &self,
        connection: &AdapterConnection,
        message: &InboundMessage,
        attachment: &InboundAttachment,
        file_id: &str,
    ) -> Result<(), String> {
        // A redelivery finds the row its first attempt wrote.
        if matches!(self.files.metadata(file_id).await, Ok(Some(_))) {
            return Ok(());
        }
        let too_large = || format!("it is larger than {} MiB", MAX_ATTACHMENT_BYTES >> 20);
        if attachment
            .size
            .is_some_and(|size| size > MAX_ATTACHMENT_BYTES)
        {
            return Err(too_large());
        }
        if !self.files.is_configured() {
            return Err("file storage is not configured".into());
        }
        let data = self
            .fetcher
            .fetch_attachment(connection, attachment, MAX_ATTACHMENT_BYTES)
            .await
            .map_err(|e| format!("it could not be downloaded ({e})"))?;
        if data.len() as u64 > MAX_ATTACHMENT_BYTES {
            return Err(too_large());
        }
        let upload = NewUpload {
            filename: attachment.filename.clone(),
            mime_type: attachment.mime_type.clone(),
            data,
            file_id: Some(file_id.to_string()),
        };
        self.files
            .store_upload(&ScopeContext::user(&message.user_id), upload)
            .await
            .map(|_| ())
            .map_err(|e| format!("it could not be stored ({e})"))
    }
}

/// The upload id for `attachment`, the same on every delivery of one event so
/// a retry finds what an earlier attempt stored.
fn upload_id(
    connection: &AdapterConnection,
    message: &InboundMessage,
    attachment: &InboundAttachment,
) -> String {
    let event = message
        .external_event_id
        .as_deref()
        .unwrap_or(&message.message_thread_id);
    let name = format!("{}:{event}:{}", connection.id, attachment.external_id);
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes()).to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
    use moatless_core::file_storage::FileInfo;
    use moatless_core::file_storage::test_support::files_from;
    use moatless_types::access_control::Scope;
    use moatless_types::adapter::{AdapterKind, Audience};

    /// Records every fetch and answers each with the same error.
    #[derive(Default)]
    struct RefusingFetcher(Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl AdapterInboundFiles for RefusingFetcher {
        async fn fetch_attachment(
            &self,
            _connection: &AdapterConnection,
            attachment: &InboundAttachment,
            _max_bytes: u64,
        ) -> Result<Vec<u8>, AdapterError> {
            self.0
                .lock()
                .expect("not poisoned")
                .push(attachment.external_id.clone());
            Err(SharedAdapterError::AdapterApi("refused".into()).into())
        }
    }

    fn connection() -> AdapterConnection {
        let now = chrono::Utc::now();
        AdapterConnection {
            id: "ac_1".into(),
            adapter_kind: AdapterKind::Slack,
            connection_kind: "app".into(),
            scope_value: "T1".into(),
            credential_ref: None,
            webhook_secret: None,
            transport_config: serde_json::json!({}),
            created_by: "user-1".into(),
            scope: Scope::Global,
            owner_user_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn attachment(id: &str, size: Option<u64>) -> InboundAttachment {
        InboundAttachment {
            external_id: id.into(),
            filename: format!("{id}.txt"),
            mime_type: "text/plain".into(),
            size,
            download_url: format!("https://files.slack.com/{id}"),
        }
    }

    fn message(text: &str, attachments: Vec<InboundAttachment>) -> InboundMessage {
        InboundMessage {
            adapter_kind: AdapterKind::Slack,
            source_id: "C1".into(),
            thread_id: None,
            message_thread_id: "1700000000.000100".into(),
            user_id: "user-1".into(),
            author: None,
            sender_external_id: None,
            requester: None,
            text: text.into(),
            file_ids: vec![],
            attachments,
            thread_properties: serde_json::Value::Null,
            adapter_context: serde_json::Value::Null,
            external_event_id: Some("slack:ts:1700000000.000100".into()),
            source_task_id: None,
            audience: Audience::Everyone,
            replies_to: Vec::new(),
            event: None,
        }
    }

    fn info(file_id: String) -> FileInfo {
        FileInfo {
            file_id,
            filename: "F1.txt".into(),
            mime_type: "text/plain".into(),
            size: 7,
        }
    }

    #[tokio::test]
    async fn a_redelivery_reuses_the_upload_its_first_attempt_stored() {
        let mut msg = message("see file", vec![attachment("F1", Some(7))]);
        let stored = upload_id(&connection(), &msg, &msg.attachments[0]);
        let fetcher = Arc::new(RefusingFetcher::default());
        let files = InboundFiles::new(fetcher.clone(), files_from(vec![info(stored.clone())]));

        files.store(&connection(), &mut msg).await;

        assert_eq!(msg.file_ids, vec![stored]);
        assert_eq!(msg.text, "see file");
        assert!(msg.attachments.is_empty());
        assert!(fetcher.0.lock().expect("not poisoned").is_empty());
    }

    /// Each file that cannot be stored becomes a note, and nothing is fetched
    /// that could not be kept.
    #[tokio::test]
    async fn a_file_that_cannot_be_stored_is_named_in_the_text_without_a_fetch() {
        let mut msg = message(
            "two files",
            vec![
                attachment("F1", Some(MAX_ATTACHMENT_BYTES + 1)),
                attachment("F2", Some(7)),
            ],
        );
        let fetcher = Arc::new(RefusingFetcher::default());
        let files = InboundFiles::new(fetcher.clone(), files_from(vec![]));

        files.store(&connection(), &mut msg).await;

        assert!(msg.file_ids.is_empty());
        assert_eq!(
            msg.text,
            "two files\n\n\
             [Attachment `F1.txt` was not delivered: it is larger than 50 MiB]\n\n\
             [Attachment `F2.txt` was not delivered: file storage is not configured]"
        );
        assert!(fetcher.0.lock().expect("not poisoned").is_empty());
    }

    /// Past the budget, each attachment left becomes a note without a lookup
    /// or a fetch, so routing still runs inside the delivery's deadline.
    #[tokio::test]
    async fn attachments_left_when_the_budget_runs_out_are_noted() {
        let mut msg = message("late", vec![attachment("F1", Some(7))]);
        let fetcher = Arc::new(RefusingFetcher::default());
        let files = InboundFiles::new(fetcher.clone(), files_from(vec![]));

        files
            .store_until(&connection(), &mut msg, tokio::time::Instant::now())
            .await;

        assert!(msg.file_ids.is_empty());
        assert_eq!(
            msg.text,
            "late\n\n[Attachment `F1.txt` was not delivered: the delivery ran out of time to fetch it]"
        );
        assert!(fetcher.0.lock().expect("not poisoned").is_empty());
    }

    #[test]
    fn the_upload_id_is_stable_per_event_and_distinct_across_events() {
        let first = message("", vec![attachment("F1", None)]);
        let mut second = first.clone();
        second.external_event_id = Some("slack:ts:1700000000.000200".into());
        let a = &first.attachments[0];

        assert_eq!(
            upload_id(&connection(), &first, a),
            upload_id(&connection(), &first, a)
        );
        assert_ne!(
            upload_id(&connection(), &first, a),
            upload_id(&connection(), &second, a)
        );
    }
}
