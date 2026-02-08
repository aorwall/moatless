//! Figma request/response types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ImportClipboardRequest {
    pub clipboard_data: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ImportNodeRequest {
    pub file_key: String,
    pub node_id: String,
    pub access_token: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FigmaImportResponse {
    pub import_id: String,
    pub prompt: String,
    pub node_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FigmaContextResponse {
    pub import_id: String,
    pub prompt: String,
    pub metadata: serde_json::Value,
}
