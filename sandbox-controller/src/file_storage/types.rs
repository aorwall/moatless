//! File storage request/response types.

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResponse {
    pub file_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
}
