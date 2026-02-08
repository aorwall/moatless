//! Figma request/response types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportClipboardRequest {
    pub clipboard_html: String,
    #[serde(default)]
    pub include_image_in_prompt: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportNodeRequest {
    /// Full Figma URL (e.g., https://www.figma.com/file/{key}/Title?node-id={id})
    pub url: String,
    #[serde(default)]
    pub include_image_in_prompt: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseClipboardRequest {
    pub clipboard_html: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FigmaImportResponse {
    pub import_id: String,
    pub file_id: Option<String>,
    pub node_id: Option<String>,
    pub file_name: Option<String>,
    pub node_name: Option<String>,
    pub has_image: bool,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FigmaImportDetailResponse {
    pub import_id: String,
    pub file_id: Option<String>,
    pub node_id: Option<String>,
    pub file_name: Option<String>,
    pub node_name: Option<String>,
    pub has_image: bool,
    pub created_at: String,
    pub prompt: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseClipboardResponse {
    pub clipboard_json: serde_json::Value,
    pub file_name: Option<String>,
    pub node_name: Option<String>,
}
