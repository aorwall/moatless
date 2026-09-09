//! Persisted Adapter connection rows and domain decoding.

use chrono::{DateTime, Utc};
use moatless_types::access_control::Scope;
use moatless_types::adapter::{AdapterConnection, AdapterKind};

/// Row from the `adapter_connections` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub(super) struct AdapterConnectionRow {
    pub id: String,
    pub adapter_kind: String,
    pub connection_kind: String,
    pub scope_value: String,
    pub credential_ref: Option<String>,
    pub webhook_secret: Option<String>,
    pub transport_config: serde_json::Value,
    pub created_by: String,
    #[sqlx(try_from = "String")]
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<AdapterConnectionRow> for AdapterConnection {
    fn from(row: AdapterConnectionRow) -> Self {
        Self {
            adapter_kind: AdapterKind::parse_or_webhook(&row.adapter_kind),
            id: row.id,
            connection_kind: row.connection_kind,
            scope_value: row.scope_value,
            credential_ref: row.credential_ref,
            webhook_secret: row.webhook_secret,
            transport_config: row.transport_config,
            created_by: row.created_by,
            scope: row.scope,
            owner_user_id: row.owner_user_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
