//! Figma HTTP handlers.
//!
//! Integrates the figma-clipboard Rust crate for clipboard parsing and
//! the Figma REST API for node imports.

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;

use crate::db::{figma_import_dao, provider_token_dao};
use crate::db::models::FigmaImportRow;
use crate::extractors::AuthUser;
use crate::file_storage::s3;
use crate::state::AppState;

use super::error::FigmaError;
use super::types::*;

/// Import a Figma design from clipboard HTML.
#[utoipa::path(
    post,
    path = "/api/v1/figma/import-clipboard",
    tag = "figma",
    request_body = ImportClipboardRequest,
    responses(
        (status = 200, description = "Import created", body = FigmaImportResponse),
        (status = 400, description = "Parse error", body = crate::sandbox::types::ErrorBody)
    )
)]
pub async fn import_clipboard(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<ImportClipboardRequest>,
) -> Result<Json<FigmaImportResponse>, FigmaError> {
    // Parse and normalize clipboard HTML
    let result = figma_clipboard::parse_and_normalize_clipboard(&req.clipboard_html)
        .map_err(|e| FigmaError::ParseError(format!("Failed to parse clipboard: {e}")))?;

    let file_name = Some(result.metadata.file_key.clone());
    let node_name = Some(result.root.name.clone());
    let file_key = Some(result.metadata.file_key.clone());

    // Serialize the normalized node tree as metadata
    let node_metadata =
        serde_json::to_value(&result.root).ok();

    let import_id = nanoid::nanoid!();
    let now = chrono::Utc::now();

    // Optionally download image if user has Figma token and wants images
    let has_image = false; // TODO: Download from Figma CDN if token available

    let row = FigmaImportRow {
        import_id: import_id.clone(),
        file_id: file_name.clone(),
        file_key,
        node_id: None,
        file_name: file_name.clone(),
        node_name: node_name.clone(),
        node_metadata,
        has_image: Some(has_image),
        include_image_in_prompt: Some(req.include_image_in_prompt),
        created_by: Some(auth.user_id),
        created_at: Some(now),
    };

    figma_import_dao::insert(&state.db, &row).await?;

    Ok(Json(FigmaImportResponse {
        import_id,
        file_id: file_name,
        node_id: None,
        file_name: row.file_name,
        node_name,
        has_image,
        created_at: now.to_rfc3339(),
    }))
}

/// Import a Figma design from a Figma URL.
#[utoipa::path(
    post,
    path = "/api/v1/figma/import-node",
    tag = "figma",
    request_body = ImportNodeRequest,
    responses(
        (status = 200, description = "Import created", body = FigmaImportResponse),
        (status = 400, description = "Bad request", body = crate::sandbox::types::ErrorBody)
    )
)]
pub async fn import_node(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<ImportNodeRequest>,
) -> Result<Json<FigmaImportResponse>, FigmaError> {
    // Parse Figma URL → extract file_key + node_id
    let (file_key, node_id) = parse_figma_url(&req.url)?;

    // Get user's Figma token
    let token = provider_token_dao::find_provider_token(&state.db, &auth.user_id, "figma")
        .await
        .map_err(FigmaError::Db)?
        .ok_or_else(|| {
            FigmaError::BadRequest(
                "No Figma token configured. Please connect your Figma account.".to_string(),
            )
        })?;

    // Call Figma REST API: GET /v1/files/{file_key}/nodes?ids={node_id}
    let client = reqwest::Client::new();
    let nodes_resp = client
        .get(format!(
            "https://api.figma.com/v1/files/{file_key}/nodes?ids={node_id}"
        ))
        .header("X-Figma-Token", &token.access_token)
        .send()
        .await
        .map_err(|e| FigmaError::ApiError(format!("Figma API request failed: {e}")))?;

    if !nodes_resp.status().is_success() {
        let status = nodes_resp.status();
        let body = nodes_resp.text().await.unwrap_or_default();
        return Err(FigmaError::ApiError(format!(
            "Figma API returned {status}: {body}"
        )));
    }

    let nodes_data: serde_json::Value = nodes_resp
        .json()
        .await
        .map_err(|e| FigmaError::ApiError(format!("Failed to parse Figma response: {e}")))?;

    let file_name = nodes_data
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Extract the node data
    let node_data = nodes_data
        .get("nodes")
        .and_then(|n| n.get(&node_id))
        .and_then(|n| n.get("document"));

    let node_name = node_data
        .and_then(|d| d.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Download image from Figma
    let mut has_image = false;
    if req.include_image_in_prompt {
        let images_resp = client
            .get(format!(
                "https://api.figma.com/v1/images/{file_key}?ids={node_id}&format=png&scale=1"
            ))
            .header("X-Figma-Token", &token.access_token)
            .send()
            .await
            .ok();

        if let Some(resp) = images_resp {
            if resp.status().is_success() {
                let images_data: serde_json::Value =
                    resp.json().await.unwrap_or(serde_json::json!({}));

                if let Some(image_url) = images_data
                    .get("images")
                    .and_then(|i| i.get(&node_id))
                    .and_then(|v| v.as_str())
                {
                    // Download the image
                    if let Ok(img_resp) = client.get(image_url).send().await {
                        if img_resp.status().is_success() {
                            if let Ok(img_bytes) = img_resp.bytes().await {
                                // Upload to S3
                                if let (Some(ref s3_client), Some(ref bucket)) =
                                    (&state.s3, &state.config.s3_bucket_name)
                                {
                                    let key = format!(
                                        "{}/figma/{}.png",
                                        state.config.s3_key_prefix,
                                        nanoid::nanoid!()
                                    );
                                    if s3::put_object(
                                        s3_client,
                                        bucket,
                                        &key,
                                        img_bytes.to_vec(),
                                        "image/png",
                                    )
                                    .await
                                    .is_ok()
                                    {
                                        has_image = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let import_id = nanoid::nanoid!();
    let now = chrono::Utc::now();

    let row = FigmaImportRow {
        import_id: import_id.clone(),
        file_id: Some(file_key.clone()),
        file_key: Some(file_key),
        node_id: Some(node_id),
        file_name: file_name.clone(),
        node_name: node_name.clone(),
        node_metadata: node_data.cloned(),
        has_image: Some(has_image),
        include_image_in_prompt: Some(req.include_image_in_prompt),
        created_by: Some(auth.user_id),
        created_at: Some(now),
    };

    figma_import_dao::insert(&state.db, &row).await?;

    Ok(Json(FigmaImportResponse {
        import_id,
        file_id: row.file_id,
        node_id: row.node_id,
        file_name,
        node_name,
        has_image,
        created_at: now.to_rfc3339(),
    }))
}

/// Parse clipboard HTML into structured Figma data.
#[utoipa::path(
    post,
    path = "/api/v1/figma/parse-clipboard",
    tag = "figma",
    request_body = ParseClipboardRequest,
    responses(
        (status = 200, description = "Parsed clipboard", body = ParseClipboardResponse),
        (status = 400, description = "Parse error", body = crate::sandbox::types::ErrorBody)
    )
)]
pub async fn parse_clipboard(
    State(_state): State<AppState>,
    _auth: AuthUser,
    Json(req): Json<ParseClipboardRequest>,
) -> Result<Json<ParseClipboardResponse>, FigmaError> {
    let result = figma_clipboard::parse_and_normalize_clipboard(&req.clipboard_html)
        .map_err(|e| FigmaError::ParseError(format!("Failed to parse clipboard: {e}")))?;

    let clipboard_json = serde_json::to_value(&result.root)
        .map_err(|e| FigmaError::ParseError(format!("Failed to serialize result: {e}")))?;

    Ok(Json(ParseClipboardResponse {
        clipboard_json,
        file_name: Some(result.metadata.file_key),
        node_name: Some(result.root.name),
    }))
}

/// Get details of a Figma import.
#[utoipa::path(
    get,
    path = "/api/v1/figma/imports/{import_id}",
    tag = "figma",
    params(
        ("import_id" = String, Path, description = "Import identifier")
    ),
    responses(
        (status = 200, description = "Import details", body = FigmaImportDetailResponse),
        (status = 404, description = "Import not found", body = crate::sandbox::types::ErrorBody)
    )
)]
pub async fn get_import(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(import_id): Path<String>,
) -> Result<Json<FigmaImportDetailResponse>, FigmaError> {
    let row = figma_import_dao::find_by_id(&state.db, &import_id)
        .await?
        .ok_or_else(|| FigmaError::NotFound(import_id))?;

    // Generate prompt from stored metadata
    let prompt = if let Some(ref metadata) = row.node_metadata {
        // Try to deserialize as NormalizedNode and generate prompt
        if let Ok(node) = serde_json::from_value::<figma_clipboard::NormalizedNode>(metadata.clone())
        {
            let config = figma_clipboard::FigmaPromptConfig::default();
            figma_clipboard::generate_figma_prompt(
                &node,
                row.file_name.as_deref().unwrap_or("unknown"),
                row.node_name.as_deref().unwrap_or("unknown"),
                &config,
            )
        } else {
            // Fallback: return raw JSON as prompt context
            format!("Figma design data:\n{}", serde_json::to_string_pretty(metadata).unwrap_or_default())
        }
    } else {
        "No design data available for this import.".to_string()
    };

    Ok(Json(FigmaImportDetailResponse {
        import_id: row.import_id,
        file_id: row.file_id,
        node_id: row.node_id,
        file_name: row.file_name,
        node_name: row.node_name,
        has_image: row.has_image.unwrap_or(false),
        created_at: row
            .created_at
            .map(|t| t.to_rfc3339())
            .unwrap_or_default(),
        prompt,
    }))
}

/// Get the image for a Figma import.
#[utoipa::path(
    get,
    path = "/api/v1/figma/imports/{import_id}/image",
    tag = "figma",
    params(
        ("import_id" = String, Path, description = "Import identifier")
    ),
    responses(
        (status = 200, description = "Image content", content_type = "image/png"),
        (status = 404, description = "Image not found", body = crate::sandbox::types::ErrorBody)
    )
)]
pub async fn get_import_image(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(import_id): Path<String>,
) -> Result<impl IntoResponse, FigmaError> {
    let row = figma_import_dao::find_by_id(&state.db, &import_id)
        .await?
        .ok_or_else(|| FigmaError::NotFound(import_id.clone()))?;

    if !row.has_image.unwrap_or(false) {
        return Err(FigmaError::NotFound(format!(
            "No image available for import {import_id}"
        )));
    }

    let (s3_client, bucket) = match (&state.s3, &state.config.s3_bucket_name) {
        (Some(client), Some(bucket)) => (client, bucket),
        _ => {
            return Err(FigmaError::StorageError(
                "S3 storage not configured".to_string(),
            ))
        }
    };

    // Look for the image in S3 by import_id pattern
    let key = format!("{}/figma/{}.png", state.config.s3_key_prefix, import_id);

    let (bytes, content_type) =
        s3::get_object(s3_client, bucket, &key)
            .await
            .map_err(|e| FigmaError::StorageError(e))?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, content_type)],
        bytes,
    ))
}

// =============================================================================
// URL parsing helpers
// =============================================================================

/// Parse a Figma URL to extract file_key and node_id.
///
/// Handles URLs like:
/// - https://www.figma.com/file/{key}/Title?node-id={id}
/// - https://www.figma.com/design/{key}/Title?node-id={id}
fn parse_figma_url(url: &str) -> Result<(String, String), FigmaError> {
    // Extract file key from path
    let path_parts: Vec<&str> = url.split('/').collect();
    let file_key = path_parts
        .iter()
        .enumerate()
        .find(|(_, &part)| part == "file" || part == "design")
        .and_then(|(i, _)| path_parts.get(i + 1))
        .ok_or_else(|| FigmaError::BadRequest(format!("Cannot extract file key from URL: {url}")))?
        .to_string();

    // Extract node-id from query parameters
    let node_id = url
        .split('?')
        .nth(1)
        .and_then(|query| {
            query.split('&').find_map(|param| {
                let (key, value) = param.split_once('=')?;
                if key == "node-id" {
                    Some(value.to_string())
                } else {
                    None
                }
            })
        })
        .ok_or_else(|| {
            FigmaError::BadRequest(format!("Cannot extract node-id from URL: {url}"))
        })?;

    // Convert dashes to colons (Figma URL encoding)
    let node_id = node_id.replace('-', ":");

    Ok((file_key, node_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_figma_file_url() {
        let url = "https://www.figma.com/file/abc123/My-Design?node-id=1-2";
        let (key, node) = parse_figma_url(url).unwrap();
        assert_eq!(key, "abc123");
        assert_eq!(node, "1:2"); // dashes converted to colons
    }

    #[test]
    fn parse_figma_design_url() {
        let url = "https://www.figma.com/design/xyz789/Title?node-id=10-20&t=abc";
        let (key, node) = parse_figma_url(url).unwrap();
        assert_eq!(key, "xyz789");
        assert_eq!(node, "10:20");
    }

    #[test]
    fn parse_figma_url_missing_file_key() {
        let url = "https://www.figma.com/other/page?node-id=1-2";
        let result = parse_figma_url(url);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot extract file key"));
    }

    #[test]
    fn parse_figma_url_missing_node_id() {
        let url = "https://www.figma.com/file/abc123/My-Design";
        let result = parse_figma_url(url);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot extract node-id"));
    }

    #[test]
    fn parse_figma_url_node_id_no_dashes() {
        let url = "https://www.figma.com/file/abc123/Title?node-id=42";
        let (key, node) = parse_figma_url(url).unwrap();
        assert_eq!(key, "abc123");
        assert_eq!(node, "42"); // no dashes to convert
    }

    #[test]
    fn parse_figma_url_complex_node_id() {
        let url = "https://www.figma.com/design/k/T?node-id=100-200-300&mode=dev";
        let (key, node) = parse_figma_url(url).unwrap();
        assert_eq!(key, "k");
        assert_eq!(node, "100:200:300");
    }
}
