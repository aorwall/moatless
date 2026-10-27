//! Methods that act on the Artifacts a thread's Task published.

use crate::authentication::viewer_scope;
use crate::oauth::Scope;
use crate::rpc::envelope::success_exit;
use crate::rpc::errors::artifact_visibility_failure_exit;
use crate::rpc::table::MethodTable;
use crate::rpc::table::Request;
use moatless_types::task::Visibility;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

pub(super) fn artifact_methods() -> MethodTable {
    MethodTable::new().session(
        "artifacts.setVisibility",
        Scope::OrchestrationOperate,
        artifacts_set_visibility,
    )
}

/// The contract's `ArtifactSetVisibilityInput`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ArtifactSetVisibilityInput {
    artifact_id: String,
    visibility: Visibility,
}

pub(super) async fn artifacts_set_visibility(
    request: Request<ArtifactSetVisibilityInput>,
) -> Vec<Value> {
    let Request {
        id,
        scope,
        payload:
            ArtifactSetVisibilityInput {
                artifact_id,
                visibility,
            },
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match state
        .artifacts
        .set_visibility(&viewer, &artifact_id, visibility)
        .await
    {
        Ok(()) => vec![success_exit(
            &id,
            json!({ "artifactId": artifact_id, "visibility": visibility }),
        )],
        Err(error) => vec![artifact_visibility_failure_exit(
            &id, scope, &error, &identity,
        )],
    }
}
