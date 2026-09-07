use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Row from the `integration_pairing_requests` table.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct PairingRequest {
    pub id: i32,
    pub loop_id: String,
    pub adapter_kind: String,
    pub provider_user_id: String,
    pub code: String,
    pub status: String,
    pub meta: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
