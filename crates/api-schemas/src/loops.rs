//! Repo "loop" sync API schemas.
//!
//! A "loop" is an agent automation declared in a repo's git tree at
//! `.moatless/loops/<loop-name>/LOOP.md` and reconciled into the trigger
//! tables by `sync-config`. These types describe the sync outcome returned to
//! the CLI / API.

use serde::{Deserialize, Serialize};

use crate::repository::RepositoryResponse;

/// One loop that was skipped during a sync, with a human-readable reason
/// (e.g. "no github connection for org acme").
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LoopSyncSkip {
    /// The loop directory name (`.moatless/loops/<loop>/`).
    pub loop_name: String,
    /// Why the loop was skipped.
    pub reason: String,
}

/// Summary of reconciling a repo's `.moatless/loops/` into the trigger tables.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LoopSyncSummary {
    /// Loops inserted as new triggers.
    pub created: u32,
    /// Existing loop triggers updated in place.
    pub updated: u32,
    /// Loop triggers removed because their directory is gone.
    pub removed: u32,
    /// Loops that were present but not synced, with reasons.
    pub skipped: Vec<LoopSyncSkip>,
}

/// Response for `POST /api/v1/repositories/{id}/sync-config`.
///
/// Wraps the updated repository plus the loop reconciliation summary.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SyncConfigResponse {
    pub repository: RepositoryResponse,
    pub loops: LoopSyncSummary,
}
