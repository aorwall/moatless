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
    /// Caller-assigned identity for this delivery. Optional, and what it buys
    /// differs sharply by whether it is supplied:
    ///
    /// - **Supplied.** Two requests carrying the same `deliveryId` are one
    ///   delivery. The second is recognised as a duplicate and does no further
    ///   work — on any replica, and after any restart.
    /// - **Absent.** A fresh identity is minted per request, so sending the same
    ///   body twice is two deliveries and the work is done twice.
    ///
    /// Either way the response means the backend owns the delivery. Only the
    /// deduplication is at the caller's discretion.
    ///
    /// It is read from the request body rather than a header because the
    /// signature covers the body only: an identifier carried outside the signed
    /// content could be altered in transit without invalidating the signature,
    /// which would either replay work as new or collapse two distinct
    /// deliveries into one.
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
/// It names the delivery, not a Task: routing has not run when this is written,
/// so no Task exists to name yet.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WebhookAccepted {
    /// The identity the delivery was admitted under — the caller's own
    /// `deliveryId`, or the one minted for it.
    pub delivery_id: String,
    pub status: WebhookAdmission,
}
