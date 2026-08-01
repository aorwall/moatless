//! Message content enricher — construct enriched content blocks from user input.

use serde_json::Value;

use super::contexts::task_message_context_to_input;
use super::models::{TaskMessage, TaskMessageContext};
use crate::config::Config;
use crate::file_storage::dao::FileDao;
use crate::task::schemas::MessageContextInput;

/// Content block for agent request.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Value>,
}

/// Build content blocks from user input + contexts + attachments.
///
/// The primary text block has `show_in_ui = true`; all context and file
/// blocks have `show_in_ui = false` (they appear only in the agent request).
pub async fn build_content_blocks(
    files: &dyn FileDao,
    config: &Config,
    text: &str,
    contexts: &[MessageContextInput],
    file_ids: &[String],
) -> Vec<ContentBlock> {
    let mut blocks = Vec::new();

    // Primary text block (visible in UI)
    blocks.push(ContentBlock {
        block_type: "text".to_string(),
        text: Some(text.to_string()),
        source: None,
    });

    // Context blocks (hidden from UI)
    for ctx in contexts {
        match ctx {
            MessageContextInput::PreviewUrl { reference_id, data } => {
                let url = data.url.as_deref().unwrap_or(reference_id);
                blocks.push(ContentBlock {
                    block_type: "text".to_string(),
                    text: Some(format!("User is currently viewing: {url}")),
                    source: None,
                });
            }
            MessageContextInput::Element { data, .. } => {
                let element_text = serde_json::to_string_pretty(&data).unwrap_or_default();
                blocks.push(ContentBlock {
                    block_type: "text".to_string(),
                    text: Some(format!("Selected UI element context:\n{element_text}")),
                    source: None,
                });
            }
            // File contexts are persisted separately and rendered into the
            // agent prompt via `file_ids` below; nothing to do here.
            MessageContextInput::File { .. } => {}
        }
    }

    // File attachment blocks (hidden from UI)
    for file_id in file_ids {
        if let Some(block) =
            build_file_attachment_block(files, file_id, &config.sandbox.file_mount_path).await
        {
            blocks.push(block);
        }
    }

    blocks
}

/// Attribution for a message one Task sent to another.
///
/// The agent has to be told where the message came from and how to answer it;
/// the person reading the transcript is looking at the Task that sent it and
/// does not. So this is applied on the way out and never stored — which also
/// means the reply instruction is regenerated from the current `source_task_id`
/// rather than frozen into a row at send time.
fn attributed_text(text: &str, source_task_id: Option<&str>) -> String {
    match source_task_id {
        Some(id) => format!(
            "Message from task `{id}`:\n\n{text}\n\n\
             To reply, run: `moat send --task {id} --text \"<your reply>\"`"
        ),
        None => text.to_string(),
    }
}

/// Rebuild the agent-facing content of a queued Message, at delivery time.
///
/// A user Message stores what the person wrote and nothing else. Everything
/// wrapped around it for the agent — the preview URL, the selected element, the
/// attribution to a sending Task, the attached files — is already persisted as
/// its own context row or column, so keeping a second, pre-rendered copy on the
/// Message only creates something that can disagree with them. This rebuilds it
/// from those rows on the way to the sandbox.
///
/// It is the read-side twin of [`build_content_blocks`], which the send path
/// used to call before writing the result down.
pub async fn build_pending_content_blocks(
    files: &dyn FileDao,
    config: &Config,
    message: &TaskMessage,
    contexts: &[TaskMessageContext],
) -> Vec<ContentBlock> {
    let text = attributed_text(
        message.message_text.as_deref().unwrap_or_default(),
        message.source_task_id.as_deref(),
    );

    // A row whose context type this build does not know is dropped, exactly as
    // the transform layer drops it — a forward-incompatible context must not
    // stop the message it decorates from reaching the agent.
    let inputs: Vec<MessageContextInput> = contexts
        .iter()
        .filter_map(task_message_context_to_input)
        .collect();

    build_content_blocks(files, config, &text, &inputs, &message.file_ids()).await
}

/// Build a content block from a file attachment reference.
///
/// `file_mount_path` is the on-disk directory the sandbox writes uploads to
/// before invoking the agent (defaults to `/mnt/uploads`). The prompt
/// advertises the resulting on-disk path so the agent can read the file
/// directly with no network round-trip.
async fn build_file_attachment_block(
    files: &dyn FileDao,
    file_id: &str,
    file_mount_path: &str,
) -> Option<ContentBlock> {
    let row = match files.find_by_id(file_id).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            tracing::warn!(file_id, "File metadata not found for attachment block");
            return None;
        }
        Err(e) => {
            tracing::warn!(file_id, error = %e, "Failed to load file metadata");
            return None;
        }
    };

    let mount = file_mount_path.trim_end_matches('/');
    Some(ContentBlock {
        block_type: "text".to_string(),
        text: Some(format!(
            "Attached file: {} ({}, {} bytes)\nSaved to: {}/{}/{}",
            row.filename, row.mime_type, row.size, mount, file_id, row.filename,
        )),
        source: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_storage::dao::MockFileDao;
    use crate::file_storage::models::FileMetadata;

    #[tokio::test]
    async fn file_attachment_block_renders_on_disk_path() {
        let mut mock = MockFileDao::new();
        mock.expect_find_by_id().returning(|file_id| {
            Ok(Some(FileMetadata {
                file_id: file_id.to_string(),
                filename: "report.pdf".to_string(),
                mime_type: "application/pdf".to_string(),
                size: 1234,
                scope: moatless_backend_core::scope::Scope::User,
                owner_user_id: Some("user-1".to_string()),
                created_by: Some("user-1".to_string()),
                uploaded_at: chrono::Utc::now(),
            }))
        });

        let block = build_file_attachment_block(&mock, "abc-123", "/mnt/uploads")
            .await
            .expect("expected attachment block");

        let text = block.text.expect("expected text");
        assert!(
            text.contains("Saved to: /mnt/uploads/abc-123/report.pdf"),
            "unexpected rendered text: {text}"
        );
        assert!(text.contains("Attached file: report.pdf"));
        assert!(text.contains("(application/pdf, 1234 bytes)"));
        assert!(
            !text.contains("Available at:"),
            "old URL hint should not appear: {text}"
        );
    }

    #[tokio::test]
    async fn file_attachment_block_trims_trailing_mount_slash() {
        let mut mock = MockFileDao::new();
        mock.expect_find_by_id().returning(|file_id| {
            Ok(Some(FileMetadata {
                file_id: file_id.to_string(),
                filename: "x.txt".to_string(),
                mime_type: "text/plain".to_string(),
                size: 1,
                scope: moatless_backend_core::scope::Scope::User,
                owner_user_id: Some("user-1".to_string()),
                created_by: Some("user-1".to_string()),
                uploaded_at: chrono::Utc::now(),
            }))
        });

        let block = build_file_attachment_block(&mock, "id", "/mnt/uploads/")
            .await
            .expect("expected attachment block");

        let text = block.text.expect("expected text");
        assert!(text.contains("/mnt/uploads/id/x.txt"));
        assert!(!text.contains("/mnt/uploads//"));
    }
}
