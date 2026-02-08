//! Agent request builder — construct enriched content blocks from user input.

use serde_json::Value;

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
pub fn build_content_blocks(
    text: &str,
    _contexts: &[crate::task::types::MessageContextInput],
    _file_ids: &[String],
) -> Vec<ContentBlock> {
    let mut blocks = Vec::new();

    // Primary text block
    blocks.push(ContentBlock {
        block_type: "text".to_string(),
        text: Some(text.to_string()),
        source: None,
    });

    // TODO: Add context blocks (Figma, preview URLs, etc.)
    // TODO: Add file attachment blocks

    blocks
}
