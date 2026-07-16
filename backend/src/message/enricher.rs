//! Message content enricher — construct enriched content blocks from user input.

use serde_json::Value;

use crate::config::Config;
use crate::figma::dao::FigmaRepository;
use crate::file_storage::dao::FileRepository;
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
    figma_imports: &dyn FigmaRepository,
    files: &dyn FileRepository,
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
            MessageContextInput::Figma { reference_id, .. } => {
                if let Some(block) = build_figma_context_block(figma_imports, reference_id).await {
                    blocks.push(block);
                }
            }
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

/// Build a content block from a Figma import reference.
async fn build_figma_context_block(
    figma_imports: &dyn FigmaRepository,
    import_id: &str,
) -> Option<ContentBlock> {
    let row = match figma_imports.find_by_id(import_id).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            tracing::warn!(import_id, "Figma import not found for context block");
            return None;
        }
        Err(e) => {
            tracing::warn!(import_id, error = %e, "Failed to load Figma import");
            return None;
        }
    };

    // Generate prompt from stored node metadata
    let prompt = {
        let metadata = row.node_metadata?;
        if let Ok(node) =
            serde_json::from_value::<figma_clipboard::NormalizedNode>(metadata.clone())
        {
            let config = figma_clipboard::FigmaPromptConfig::default();
            figma_clipboard::generate_figma_prompt(
                &node,
                row.file_name.as_deref().unwrap_or("unknown"),
                row.node_name.as_deref().unwrap_or("unknown"),
                &config,
            )
        } else {
            format!(
                "Figma design data:\n{}",
                serde_json::to_string_pretty(&metadata).unwrap_or_default()
            )
        }
    };

    Some(ContentBlock {
        block_type: "text".to_string(),
        text: Some(prompt),
        source: None,
    })
}

/// Build a content block from a file attachment reference.
///
/// `file_mount_path` is the on-disk directory the sandbox writes uploads to
/// before invoking the agent (defaults to `/mnt/uploads`). The prompt
/// advertises the resulting on-disk path so the agent can read the file
/// directly with no network round-trip.
async fn build_file_attachment_block(
    files: &dyn FileRepository,
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
    use crate::file_storage::dao::MockFileRepository;
    use crate::file_storage::models::FileMetadata;

    #[tokio::test]
    async fn file_attachment_block_renders_on_disk_path() {
        let mut mock = MockFileRepository::new();
        mock.expect_find_by_id().returning(|file_id| {
            Ok(Some(FileMetadata {
                file_id: file_id.to_string(),
                filename: "report.pdf".to_string(),
                mime_type: "application/pdf".to_string(),
                size: 1234,
                scope: moatless_backend_core::scope::Scope::User,
                team_id: "personal-user-1".to_string(),
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
        let mut mock = MockFileRepository::new();
        mock.expect_find_by_id().returning(|file_id| {
            Ok(Some(FileMetadata {
                file_id: file_id.to_string(),
                filename: "x.txt".to_string(),
                mime_type: "text/plain".to_string(),
                size: 1,
                scope: moatless_backend_core::scope::Scope::User,
                team_id: "personal-user-1".to_string(),
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
