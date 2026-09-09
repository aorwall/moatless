//! Adapter app rows, encrypted secrets and carry-over transactions.

use crate::adapter_apps::models::{
    AdapterAppRow, AdapterAppSecretRow, SealedAdapterApp, StoredAdapterApp,
};
use crate::adapter_apps::service::app_id;
use crate::auth::error::AuthError;
use moatless_core::db::DbPool;
use serde_json::{Value, json};
use sqlx::Row;
use std::collections::HashMap;
use tracing::{info, warn};
const SYSTEM_ACTOR: &str = "adapter_apps_env_carry_over";

pub(in crate::adapter_apps) async fn has_marker(
    db: &DbPool,
    marker: &str,
) -> Result<bool, AuthError> {
    let row: (bool,) =
        sqlx::query_as("SELECT EXISTS (SELECT 1 FROM adapter_app_migrations WHERE name = $1)")
            .bind(marker)
            .fetch_one(db)
            .await?;
    Ok(row.0)
}

pub(in crate::adapter_apps) async fn list_adapter_apps(
    db: &DbPool,
) -> Result<Vec<StoredAdapterApp>, AuthError> {
    let rows: Vec<AdapterAppRow> = sqlx::query_as(
        r#"SELECT id, adapter_kind, app_key, display_name, config_json, revision, updated_at
           FROM adapter_apps
           ORDER BY adapter_kind, app_key"#,
    )
    .fetch_all(db)
    .await?;

    let mut apps = Vec::with_capacity(rows.len());
    for row in rows {
        apps.push(stored_app_from_row(db, row).await?);
    }
    Ok(apps)
}

pub(in crate::adapter_apps) async fn find_adapter_app(
    db: &DbPool,
    adapter_kind: &str,
    app_key: &str,
) -> Result<Option<StoredAdapterApp>, AuthError> {
    let row: Option<AdapterAppRow> = sqlx::query_as(
        r#"SELECT id, adapter_kind, app_key, display_name, config_json, revision, updated_at
           FROM adapter_apps
           WHERE adapter_kind = $1 AND app_key = $2"#,
    )
    .bind(adapter_kind)
    .bind(app_key)
    .fetch_optional(db)
    .await?;
    match row {
        Some(row) => Ok(Some(stored_app_from_row(db, row).await?)),
        None => Ok(None),
    }
}

async fn stored_app_from_row(
    db: &DbPool,
    row: AdapterAppRow,
) -> Result<StoredAdapterApp, AuthError> {
    let secret_fingerprints: HashMap<String, String> =
        sqlx::query("SELECT secret_fingerprints FROM adapter_apps WHERE id = $1")
            .bind(&row.id)
            .fetch_one(db)
            .await?
            .try_get::<Value, _>("secret_fingerprints")
            .unwrap_or_else(|_| json!({}))
            .as_object()
            .map(|object| {
                object
                    .iter()
                    .filter_map(|(key, value)| value.as_str().map(|v| (key.clone(), v.to_string())))
                    .collect()
            })
            .unwrap_or_default();
    Ok(StoredAdapterApp {
        id: row.id,
        adapter_kind: row.adapter_kind,
        app_key: row.app_key,
        display_name: row.display_name,
        config_json: row.config_json,
        secret_fingerprints,
        revision: row.revision,
        updated_at: row.updated_at,
    })
}

pub(in crate::adapter_apps) async fn delete_adapter_app(
    db: &DbPool,
    adapter_kind: &str,
    app_key: &str,
) -> Result<bool, AuthError> {
    let deleted = sqlx::query("DELETE FROM adapter_apps WHERE adapter_kind = $1 AND app_key = $2")
        .bind(adapter_kind)
        .bind(app_key)
        .execute(db)
        .await?;
    Ok(deleted.rows_affected() > 0)
}

async fn insert_app(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    input: &SealedAdapterApp,
    actor_id: &str,
) -> Result<bool, AuthError> {
    let id = app_id(input.adapter_kind, &input.app_key);
    let inserted = sqlx::query(
        r#"INSERT INTO adapter_apps (
               id, adapter_kind, app_key, display_name, enabled, config_json,
               secret_fingerprints, created_by, updated_by, revision
           )
           VALUES ($1, $2, $3, $4, true, $5, $6, $7, $7, 1)
           ON CONFLICT (adapter_kind, app_key) DO NOTHING"#,
    )
    .bind(&id)
    .bind(input.adapter_kind)
    .bind(&input.app_key)
    .bind(&input.display_name)
    .bind(&input.config_json)
    .bind(&input.fingerprints)
    .bind(actor_id)
    .execute(&mut **tx)
    .await?;

    if inserted.rows_affected() == 0 {
        return Ok(false);
    }

    for (name, encrypted) in &input.secrets {
        sqlx::query(
            r#"INSERT INTO adapter_app_secrets (
                   adapter_app_id, name, value_ciphertext, value_nonce,
                   encrypted_data_key, kms_key_id, encryption_version
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        )
        .bind(&id)
        .bind(name)
        .bind(&encrypted.value_ciphertext)
        .bind(&encrypted.encryption_nonce)
        .bind(&encrypted.encrypted_data_key)
        .bind(&encrypted.kms_key_id)
        .bind(encrypted.encryption_version)
        .execute(&mut **tx)
        .await?;
    }
    Ok(true)
}

pub(in crate::adapter_apps) async fn upsert(
    db: &DbPool,
    input: SealedAdapterApp,
    actor_id: &str,
) -> Result<StoredAdapterApp, AuthError> {
    let id = app_id(input.adapter_kind, &input.app_key);
    let mut tx = db.begin().await?;
    sqlx::query(
        r#"INSERT INTO adapter_apps (
               id, adapter_kind, app_key, display_name, enabled, config_json,
               secret_fingerprints, created_by, updated_by, revision
           )
           VALUES ($1, $2, $3, $4, true, $5, $6, $7, $7, 1)
           ON CONFLICT (adapter_kind, app_key) DO UPDATE SET
               display_name = EXCLUDED.display_name,
               enabled = true,
               config_json = EXCLUDED.config_json,
               secret_fingerprints = EXCLUDED.secret_fingerprints,
               updated_by = EXCLUDED.updated_by,
               updated_at = now(),
               revision = adapter_apps.revision + 1"#,
    )
    .bind(&id)
    .bind(input.adapter_kind)
    .bind(&input.app_key)
    .bind(&input.display_name)
    .bind(&input.config_json)
    .bind(&input.fingerprints)
    .bind(actor_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM adapter_app_secrets WHERE adapter_app_id = $1")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    for (name, encrypted) in &input.secrets {
        sqlx::query(
            r#"INSERT INTO adapter_app_secrets (
                   adapter_app_id, name, value_ciphertext, value_nonce,
                   encrypted_data_key, kms_key_id, encryption_version
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        )
        .bind(&id)
        .bind(name)
        .bind(&encrypted.value_ciphertext)
        .bind(&encrypted.encryption_nonce)
        .bind(&encrypted.encrypted_data_key)
        .bind(&encrypted.kms_key_id)
        .bind(encrypted.encryption_version)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    find_adapter_app(db, input.adapter_kind, &input.app_key)
        .await?
        .ok_or_else(|| AuthError::Internal("saved Adapter app was not readable".to_string()))
}

pub(in crate::adapter_apps) async fn read_secrets(
    db: &DbPool,
    adapter_kind: &str,
    app_key: &str,
) -> Result<Option<(AdapterAppRow, Vec<AdapterAppSecretRow>)>, AuthError> {
    let row: Option<AdapterAppRow> = sqlx::query_as(
        r#"SELECT id, adapter_kind, app_key, display_name, config_json, revision, updated_at
           FROM adapter_apps
           WHERE adapter_kind = $1 AND app_key = $2 AND enabled = true"#,
    )
    .bind(adapter_kind)
    .bind(app_key)
    .fetch_optional(db)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };

    let secret_rows: Vec<AdapterAppSecretRow> = sqlx::query_as(
        r#"SELECT name, value_ciphertext, value_nonce, encrypted_data_key,
                  kms_key_id, encryption_version
           FROM adapter_app_secrets
           WHERE adapter_app_id = $1"#,
    )
    .bind(&row.id)
    .fetch_all(db)
    .await?;
    Ok(Some((row, secret_rows)))
}

pub(in crate::adapter_apps) async fn carry_over(
    db: &DbPool,
    marker: &str,
    input: SealedAdapterApp,
) {
    let mut tx = match db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            warn!(error = %e, "Could not start Adapter app carry-over transaction");
            return;
        }
    };
    let claimed = match sqlx::query(
        r#"INSERT INTO adapter_app_migrations (name, detail)
           VALUES ($1, $2)
           ON CONFLICT (name) DO NOTHING"#,
    )
    .bind(marker)
    .bind(format!(
        "{}:{} carried over from environment",
        input.adapter_kind, input.app_key
    ))
    .execute(&mut *tx)
    .await
    {
        Ok(done) => done.rows_affected() > 0,
        Err(e) => {
            warn!(marker, error = %e, "Could not claim Adapter app carry-over marker");
            let _ = tx.rollback().await;
            return;
        }
    };
    if !claimed {
        let _ = tx.rollback().await;
        return;
    }

    match insert_app(&mut tx, &input, SYSTEM_ACTOR).await {
        Ok(inserted) => {
            if let Err(e) = tx.commit().await {
                warn!(marker, error = %e, "Could not commit Adapter app carry-over");
                return;
            }
            if inserted {
                info!(
                    adapter_kind = input.adapter_kind,
                    app_key = %input.app_key,
                    "Carried Adapter app env config into DB"
                );
            } else {
                warn!(
                    adapter_kind = input.adapter_kind,
                    app_key = %input.app_key,
                    "Skipped Adapter app env carry-over because that app is already registered"
                );
            }
        }
        Err(e) => {
            warn!(
                marker,
                adapter_kind = input.adapter_kind,
                app_key = %input.app_key,
                error = %e,
                "Could not carry Adapter app env config into DB"
            );
            let _ = tx.rollback().await;
        }
    }
}
