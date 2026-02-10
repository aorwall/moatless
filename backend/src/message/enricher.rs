//! Message content enricher — construct enriched content blocks from user input.

use serde_json::Value;

use crate::config::Config;
use crate::db::{figma_import_dao, file_meta_dao, DbPool};
use crate::task::types::MessageContextInput;

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
    db: &DbPool,
    _config: &Config,
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
        match ctx.context_type.as_str() {
            "figma" => {
                if let Some(block) = build_figma_context_block(db, &ctx.reference_id).await {
                    blocks.push(block);
                }
            }
            "previewUrl" => {
                let url = ctx
                    .data
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&ctx.reference_id);
                blocks.push(ContentBlock {
                    block_type: "text".to_string(),
                    text: Some(format!("User is currently viewing: {url}")),
                    source: None,
                });
            }
            "element" => {
                let element_text = serde_json::to_string_pretty(&ctx.data)
                    .unwrap_or_else(|_| ctx.data.to_string());
                blocks.push(ContentBlock {
                    block_type: "text".to_string(),
                    text: Some(format!("Selected UI element context:\n{element_text}")),
                    source: None,
                });
            }
            _ => {
                tracing::debug!(
                    context_type = %ctx.context_type,
                    "Unknown context type, skipping"
                );
            }
        }
    }

    // File attachment blocks (hidden from UI)
    for file_id in file_ids {
        if let Some(block) = build_file_attachment_block(db, file_id).await {
            blocks.push(block);
        }
    }

    blocks
}

/// Build a content block from a Figma import reference.
async fn build_figma_context_block(db: &DbPool, import_id: &str) -> Option<ContentBlock> {
    let row = match figma_import_dao::find_by_id(db, import_id).await {
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
    let prompt = if let Some(ref metadata) = row.node_metadata {
        if let Ok(node) = serde_json::from_value::<figma_clipboard::NormalizedNode>(metadata.clone())
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
                serde_json::to_string_pretty(metadata).unwrap_or_default()
            )
        }
    } else {
        return None;
    };

    Some(ContentBlock {
        block_type: "text".to_string(),
        text: Some(prompt),
        source: None,
    })
}

/// Build a content block from a file attachment reference.
async fn build_file_attachment_block(db: &DbPool, file_id: &str) -> Option<ContentBlock> {
    let row = match file_meta_dao::find_by_id(db, file_id).await {
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

    Some(ContentBlock {
        block_type: "text".to_string(),
        text: Some(format!(
            "Attached file: {} ({}, {} bytes)\nAvailable at: /api/v1/uploads/{}",
            row.filename, row.mime_type, row.size, file_id
        )),
        source: None,
    })
}
