use super::dao::{find_by_code, pairing_store_error};
use super::models::PairingRequest;
use crate::config::TelegramAdapterConfig;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_core::db::DbPool;
use moatless_loops::loops::operations::Loops;
use tracing::info;

/// List all pending pairing requests for loops owned by the given user.
pub async fn list_pending_for_user(
    db: &DbPool,
    loops: &Loops,
    user_id: &str,
) -> Result<Vec<PairingRequest>, AdapterError> {
    // The owner filter is the Loop domain's fact, so it is asked for rather
    // than joined: this crate owns `integration_pairing_requests` and nothing
    // else.
    let loop_ids = loops
        .ids_created_by(user_id)
        .await
        .map_err(pairing_store_error)?;

    super::dao::pending_for_loops(db, &loop_ids).await
}

/// Approve a pairing request: mark approved + add user ID to `dm_allowlist`.
///
/// Approved is written before the allowlist write, so a failed config write
/// leaves the request approved but the user not allowed.
pub async fn approve_request(
    db: &DbPool,
    store: &Loops,
    loop_id: &str,
    code: &str,
) -> Result<PairingRequest, AdapterError> {
    // Find the pending request
    let req = find_by_code(db, loop_id, code).await?.ok_or_else(|| {
        SharedAdapterError::BadRequest("Pairing code not found or already used".into())
    })?;

    super::dao::mark_approved(db, req.id).await?;

    // Read current adapter_config and append user to dm_allowlist
    let current_config = store
        .adapter_config(loop_id)
        .await
        .map_err(pairing_store_error)?;

    let mut tg_config: TelegramAdapterConfig = current_config
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Add user if not already present
    let user_id_str = req.provider_user_id.clone();
    if !tg_config.dm_allowlist.contains(&user_id_str) {
        tg_config.dm_allowlist.push(user_id_str);
    }

    let updated_config =
        serde_json::to_value(&tg_config).map_err(SharedAdapterError::ConfigSerialization)?;

    store
        .set_adapter_config(loop_id, &updated_config)
        .await
        .map_err(pairing_store_error)?;

    info!(
        loop_id,
        code,
        provider_user_id = %req.provider_user_id,
        "Pairing request approved, user added to dm_allowlist"
    );

    let updated = super::dao::find_by_id(db, req.id).await?;
    Ok(updated)
}
