//! File storage request/response types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UploadResponse {
    pub file_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
}

/// Doc-only type for multipart upload body.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UploadFileBody {
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "binary"))]
    pub file: Vec<u8>,
}
