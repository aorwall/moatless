//! Adapter connection response projection and query filter.

use moatless_api_schemas::adapters::AdapterConnectionResponse;
use moatless_types::adapter::{AdapterConnection, AdapterKind};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterFilter {
    pub adapter_kind: Option<AdapterKind>,
}

pub fn adapter_connection_response(row: &AdapterConnection) -> AdapterConnectionResponse {
    AdapterConnectionResponse {
        id: row.id.clone(),
        adapter_kind: row.adapter_kind,
        connection_kind: row.connection_kind.clone(),
        external_account_id: row.scope_value.clone(),
        created_by: row.created_by.clone(),
        scope: row.scope,
        owner_user_id: row.owner_user_id.clone(),
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    }
}
