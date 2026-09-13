//! Generic webhook HTTP request and acknowledgement contracts.

use serde::{Deserialize, Serialize};

/// Inbound webhook payload.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WebhookPayload {
    /// Message text (required).
    pub text: String,
    /// Thread ID for routing.
    pub thread_id: Option<String>,
    /// User identity override (defaults to config's default `user_id`).
    pub user_id: Option<String>,
    /// Arbitrary metadata stored in thread properties.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub metadata: Option<serde_json::Value>,
    /// Caller-assigned identity for this delivery, used for deduplication.
    ///
    /// Supplied: repeated `deliveryId` values collapse into one delivery, done
    /// once. Absent: each request is a new delivery. Read from the body, not a
    /// header, since only the body is signed.
    pub delivery_id: Option<String>,
}

/// What admission did with a delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum WebhookAdmission {
    /// The backend has just taken ownership of this delivery.
    Admitted,
    /// It already owned it. Nothing extra was written and nothing extra runs.
    Duplicate,
}

/// The answer to an accepted delivery.
///
/// Names the delivery, not a Task — routing hasn't run yet, so no Task exists.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WebhookAccepted {
    /// The identity the delivery was admitted under — the caller's own
    /// `deliveryId`, or the one minted for it.
    pub delivery_id: String,
    pub status: WebhookAdmission,
}
