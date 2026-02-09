//! File storage request/response types.

use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UploadResponse {
    pub file_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
}

/// Doc-only type for multipart upload body.
#[derive(ToSchema)]
pub struct UploadFileBody {
    #[schema(value_type = String, format = "binary")]
    pub file: Vec<u8>,
}
