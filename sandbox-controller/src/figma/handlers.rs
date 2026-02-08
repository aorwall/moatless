//! Figma HTTP handlers.
//!
//! TODO: Wire up the figma-clipboard crate when the API is stabilized.
//! For now, these are placeholder handlers that return errors.

use axum::extract::{Path, State};
use axum::Json;

use crate::extractors::AuthUser;
use crate::state::AppState;

use super::error::FigmaError;
use super::types::*;

/// POST /api/v1/figma/import-clipboard
pub async fn import_clipboard(
    State(_state): State<AppState>,
    _auth: AuthUser,
    Json(_req): Json<ImportClipboardRequest>,
) -> Result<Json<FigmaImportResponse>, FigmaError> {
    // TODO: Integrate figma-clipboard crate:
    // 1. Parse clipboard data
    // 2. Normalize tree
    // 3. Generate prompt
    // 4. Store result
    // 5. Optionally upload images to S3

    Err(FigmaError::BadRequest(
        "Figma clipboard import not yet implemented in Rust".to_string(),
    ))
}

/// POST /api/v1/figma/import-node
pub async fn import_node(
    State(_state): State<AppState>,
    _auth: AuthUser,
    Json(_req): Json<ImportNodeRequest>,
) -> Result<Json<FigmaImportResponse>, FigmaError> {
    // TODO: Call Figma REST API with OAuth token
    // 1. GET /v1/files/{file_key}/nodes?ids={node_id}
    // 2. Parse response
    // 3. Generate prompt
    // 4. Store result

    Err(FigmaError::BadRequest(
        "Figma node import not yet implemented in Rust".to_string(),
    ))
}

/// GET /api/v1/figma/context/{import_id}
pub async fn get_figma_context(
    State(_state): State<AppState>,
    _auth: AuthUser,
    Path(_import_id): Path<String>,
) -> Result<Json<FigmaContextResponse>, FigmaError> {
    // TODO: Return stored prompt + metadata

    Err(FigmaError::NotFound(
        "Figma context storage not yet implemented in Rust".to_string(),
    ))
}
