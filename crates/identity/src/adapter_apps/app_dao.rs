//! Adapter app rows and their encrypted secrets.

use crate::adapter_apps::models::{
    AdapterAppRow, AdapterAppSecretRow, SealedAdapterApp, StoredAdapterApp,
};
use crate::adapter_apps::service::app_id;
use crate::auth::error::AuthError;
use moatless_core::db::DbPool;
use serde_json::{Value, json};
use sqlx::Row;
use std::collections::HashMap;

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
