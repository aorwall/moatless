//! Tag response types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TagResponse {
    pub tag_id: String,
    pub name: String,
    pub description: Option<String>,
}
