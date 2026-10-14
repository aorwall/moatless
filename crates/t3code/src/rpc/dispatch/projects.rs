//! Methods that read and write the Workspace's files and attachments.

use super::orchestration_v2::command_context;
use crate::assets;
use crate::attachments;
use crate::authentication::viewer_scope;
use crate::commands;
use crate::files;
use crate::oauth::Scope;
use crate::projection;
use crate::rpc::envelope::failure_exit;
use crate::rpc::envelope::success_exit;
use crate::rpc::envelope::typed_failure_exit;
use crate::rpc::errors::file_failure_exit;
use crate::rpc::session::SEARCH_ENTRIES_LIMIT;
use crate::rpc::session::SEARCH_ENTRIES_MAX;
use crate::rpc::table::MethodTable;
use crate::rpc::table::Request;
use crate::snapshots::workspace_cwd;
use crate::v2_commands;
use crate::workspace::WorkspaceAccessError;
use serde_json::Value;
use serde_json::json;

pub(super) fn project_methods() -> MethodTable {
    MethodTable::new()
        .session(
            "assets.createUrl",
            Scope::OrchestrationRead,
            assets_create_url,
        )
        .session(
            "attachments.createUploadUrl",
            Scope::OrchestrationOperate,
            attachments_create_upload_url,
        )
        .session(
            "attachments.delete",
            Scope::OrchestrationOperate,
            attachments_delete,
        )
        .declared("projects.add", Scope::OrchestrationOperate)
        .detached(
            "projects.listEntries",
            Scope::OrchestrationRead,
            projects_list_entries,
        )
        .session(
            "projects.mutate",
            Scope::OrchestrationOperate,
            projects_mutate,
        )
        .detached(
            "projects.readFile",
            Scope::OrchestrationRead,
            projects_read_file,
        )
        .declared("projects.remove", Scope::OrchestrationOperate)
        .detached(
            "projects.searchEntries",
            Scope::OrchestrationRead,
            projects_search_entries,
        )
        .session(
            "projects.writeFile",
            Scope::OrchestrationOperate,
            projects_write_file,
        )
}

pub(super) async fn projects_list_entries(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some(cwd) = workspace_cwd(&payload) else {
        return vec![file_failure_exit(
            &id,
            tag,
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    match files::list_entries(
        &state.reads,
        state.file_sources(),
        &viewer_scope(&identity),
        &cwd,
    )
    .await
    {
        Ok(listing) => vec![success_exit(&id, listing)],
        Err(error) => vec![file_failure_exit(&id, tag, &error, &identity)],
    }
}

pub(super) async fn projects_read_file(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let relative_path = payload
        .get("relativePath")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty());
    let (Some(cwd), Some(relative_path)) = (workspace_cwd(&payload), relative_path) else {
        return vec![file_failure_exit(
            &id,
            tag,
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    match files::read_file(
        &state.reads,
        state.file_sources(),
        &viewer_scope(&identity),
        &cwd,
        relative_path,
    )
    .await
    {
        Ok(file) => vec![success_exit(&id, file)],
        Err(files::ReadFileError::Access(error)) => {
            vec![file_failure_exit(&id, tag, &error, &identity)]
        }
        Err(files::ReadFileError::Binary { path }) => vec![typed_failure_exit(
            &id,
            json!({
                "_tag": "ProjectReadFileError",
                "failure": "binary_file",
                "relativePath": path,
                "message": format!("{path} is a binary file, so it cannot be shown as text."),
            }),
        )],
    }
}

pub(super) async fn projects_search_entries(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    // An absent or blank query is legal and means "show me what is here": the
    // picker opens before anyone types. `search_entries` answers that with a
    // bounded listing rather than nothing, because the Sandbox's name search
    // matches no path at all for an empty needle and an empty picker looks like
    // a broken one.
    let query = payload
        .get("query")
        .and_then(Value::as_str)
        .map_or("", str::trim);
    let Some(cwd) = workspace_cwd(&payload) else {
        return vec![file_failure_exit(
            &id,
            tag,
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    let limit = payload
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(SEARCH_ENTRIES_LIMIT, |limit| {
            (limit as usize).min(SEARCH_ENTRIES_MAX)
        });
    match files::search_entries(
        &state.reads,
        state.file_sources(),
        &viewer_scope(&identity),
        &cwd,
        query,
        limit,
        payload.get("kind").and_then(Value::as_str),
    )
    .await
    {
        Ok(entries) => vec![success_exit(&id, entries)],
        Err(error) => vec![file_failure_exit(&id, tag, &error, &identity)],
    }
}

pub(super) async fn projects_write_file(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let relative_path = payload
        .get("relativePath")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty());
    // Empty contents are a legal write — truncating a file is a thing a person
    // does — so only an absent `contents` is unaddressable.
    let contents = payload.get("contents").and_then(Value::as_str);
    let (Some(cwd), Some(relative_path), Some(contents)) =
        (workspace_cwd(&payload), relative_path, contents)
    else {
        return vec![file_failure_exit(
            &id,
            tag,
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    match files::write_file(
        &state.reads,
        state.sandboxes.as_ref(),
        &viewer_scope(&identity),
        &cwd,
        relative_path,
        contents,
    )
    .await
    {
        Ok(written) => vec![success_exit(&id, written)],
        Err(error) => vec![file_failure_exit(&id, tag, &error, &identity)],
    }
}

/// Mints an asset as a route this backend serves the file on, not a local path
/// — a checkout lives only in its Sandbox; a favicon names no file and always
/// answers none.
pub(super) async fn assets_create_url(request: Request) -> Vec<Value> {
    let Request { id, payload, .. } = request;
    // The error carries the resource back, and the client decodes it against the
    // same schema it sent — so a request without one has no typed answer to give
    // and is a malformed request instead.
    let Some(resource) = payload.get("resource") else {
        return vec![failure_exit(&id, "assets.createUrl requires a resource")];
    };
    match assets::create_url(resource) {
        Ok(minted) => vec![success_exit(&id, minted)],
        Err(error) => vec![typed_failure_exit(
            &id,
            json!({
                "_tag": error.tag(),
                "resource": resource
            }),
        )],
    }
}

/// Mints the URL the composer uploads a file to before sending the turn that
/// names it; the URL carries its own credential, so nothing about the upload
/// reaches this socket except the id.
pub(super) async fn attachments_create_upload_url(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    match attachments::create_upload_url(&identity, state.session_jwt_secret.as_deref(), &payload) {
        Ok(minted) => vec![success_exit(&id, minted)],
        Err(attachments::MintError::Invalid(message)) => vec![failure_exit(&id, message)],
        Err(attachments::MintError::NoSigningKey) => vec![typed_failure_exit(
            &id,
            json!({
                "_tag": "AttachmentUploadSigningKeyError",
                "cause": "This deployment cannot sign attachment upload URLs."
            }),
        )],
    }
}

pub(super) async fn attachments_delete(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    match attachments::delete(
        state.attachments.as_ref(),
        &viewer_scope(&identity),
        &payload,
    )
    .await
    {
        Ok(()) => vec![success_exit(&id, Value::Null)],
        Err(message) => vec![failure_exit(&id, message)],
    }
}

/// `projects.mutate`: a project is a Moatless Workspace, so only an update of its
/// Scripts is carried out, through the V1 command
/// ([`v2_commands::project_mutation`]). Answers the updated `Project`.
pub(super) async fn projects_mutate(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let refusal = |message: String| {
        vec![typed_failure_exit(
            &id,
            json!({
                "_tag": "ProjectMutationError",
                "commandId": payload.get("commandId").cloned().unwrap_or_else(|| json!("")),
                "message": message,
            }),
        )]
    };
    let written = match v2_commands::project_mutation(&payload) {
        Ok(command) => {
            commands::dispatch(&command_context(&state, &identity), &viewer, &command).await
        }
        Err(refused) => Err(refused),
    };
    if let Err(refused) = written {
        return refusal(refused.message());
    }

    let project_id = payload.get("projectId").and_then(Value::as_str);
    let listed = state
        .reads
        .workspaces()
        .list_readable_including(&viewer, &[])
        .await;
    let Some(workspace) = listed.ok().and_then(|workspaces| {
        workspaces
            .into_iter()
            .find(|listed| Some(listed.workspace.id.as_str()) == project_id)
    }) else {
        return refusal("The project was saved but could not be read back.".to_string());
    };
    let mut project = projection::project(&workspace);
    // The shell's project omits it; `Project` requires it, and a listed
    // Workspace is live.
    project["deletedAt"] = Value::Null;
    vec![success_exit(&id, project)]
}
