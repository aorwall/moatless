//! Methods that read and write the Workspace's files and attachments.

use super::orchestration_v2::command_context;
use crate::assets;
use crate::attachments;
use crate::attachments::AttachmentCreateUploadUrlInput;
use crate::attachments::AttachmentDeleteInput;
use crate::authentication::viewer_scope;
use crate::commands;
use crate::files;
use crate::oauth::Scope;
use crate::projection;
use crate::rpc::envelope::failure_exit;
use crate::rpc::envelope::success_exit;
use crate::rpc::envelope::typed_failure_exit;
use crate::rpc::errors::directory_failure_exit;
use crate::rpc::errors::file_failure_exit;
use crate::rpc::metrics as rpc_metrics;
use crate::rpc::session::SEARCH_ENTRIES_LIMIT;
use crate::rpc::session::SEARCH_ENTRIES_MAX;
use crate::rpc::table::MethodTable;
use crate::rpc::table::Request;
use crate::snapshots::workspace_cwd;
use crate::v2_commands;
use crate::workspace::WorkspaceAccessError;
use serde::Deserialize;
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
        .detached(
            "filesystem.getMetadata",
            Scope::OrchestrationRead,
            filesystem_get_metadata,
        )
        .declared("projects.add", Scope::OrchestrationOperate)
        .detached(
            "projects.goToDefinition",
            Scope::OrchestrationRead,
            projects_go_to_definition,
        )
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
            "projects.searchContents",
            Scope::OrchestrationRead,
            projects_search_contents,
        )
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

/// The contract's `ProjectListEntriesInput`.
///
/// This group's file inputs leave optional what the contract requires, so a
/// request missing one gets the typed refusal `file_failure_exit` gives
/// [`WorkspaceAccessError::Unaddressable`] rather than a decode Die.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectListEntriesInput {
    cwd: Option<String>,
    directory_path: Option<String>,
}

/// The contract's `ProjectSearchContentsInput`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectSearchContentsInput {
    cwd: Option<String>,
    query: Option<String>,
    limit: Option<u64>,
    #[serde(default)]
    case_sensitive: bool,
    #[serde(default)]
    whole_word: bool,
    #[serde(default)]
    use_regex: bool,
}

/// The contract's `FilesystemGetMetadataInput`.
#[derive(Deserialize)]
pub(super) struct FilesystemGetMetadataInput {
    #[serde(default)]
    paths: Vec<String>,
}

/// The contract's `ProjectGoToDefinitionInput`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectGoToDefinitionInput {
    cwd: Option<String>,
    relative_path: Option<String>,
    line: Option<u64>,
    column: Option<u64>,
}

/// The contract's `ProjectReadFileInput`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectReadFileInput {
    cwd: Option<String>,
    relative_path: Option<String>,
}

/// The contract's `ProjectSearchEntriesInput`.
#[derive(Deserialize)]
pub(super) struct ProjectSearchEntriesInput {
    cwd: Option<String>,
    query: Option<String>,
    limit: Option<u64>,
    kind: Option<String>,
}

/// The contract's `ProjectWriteFileInput`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectWriteFileInput {
    cwd: Option<String>,
    relative_path: Option<String>,
    contents: Option<String>,
}

/// The contract's `AssetCreateUrlInput`.
#[derive(Deserialize)]
pub(super) struct AssetCreateUrlInput {
    /// An open union: [`assets::create_url`] answers a tag it does not serve
    /// with a typed refusal, which an enum here would turn into a decode Die.
    resource: Value,
}

/// How many content matches one search answers when the client does not say,
/// and the contract's `PROJECT_SEARCH_CONTENTS_MAX_LIMIT`.
const SEARCH_CONTENTS_LIMIT: usize = 100;
const SEARCH_CONTENTS_MAX: usize = 500;

/// The contract's `FILESYSTEM_METADATA_BATCH_LIMIT`.
const FILESYSTEM_METADATA_BATCH_LIMIT: usize = 64;

/// `relative_path` trimmed, or `None` when that leaves nothing.
fn relative_path(relative_path: Option<&str>) -> Option<&str> {
    relative_path.map(str::trim).filter(|path| !path.is_empty())
}

pub(super) async fn projects_list_entries(request: Request<ProjectListEntriesInput>) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some(cwd) = workspace_cwd(payload.cwd.as_deref()) else {
        return vec![file_failure_exit(
            &id,
            tag,
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    let directory = payload.directory_path.as_deref().map(str::trim);
    match files::list_entries(
        &state.reads,
        state.file_sources(),
        &viewer_scope(&identity),
        &cwd,
        directory,
    )
    .await
    {
        Ok(listing) => vec![success_exit(&id, listing)],
        Err(error) if directory.is_some() => {
            vec![directory_failure_exit(&id, tag, &error, &identity)]
        }
        Err(error) => vec![file_failure_exit(&id, tag, &error, &identity)],
    }
}

/// Answered only from a running Sandbox; see [`files::search_contents`].
pub(super) async fn projects_search_contents(
    request: Request<ProjectSearchContentsInput>,
) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    // Untrimmed: whitespace in a content query is part of what is searched.
    let query = payload.query.as_deref().filter(|query| !query.is_empty());
    let (Some(cwd), Some(query)) = (workspace_cwd(payload.cwd.as_deref()), query) else {
        return vec![file_failure_exit(
            &id,
            tag,
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    let limit = payload.limit.map_or(SEARCH_CONTENTS_LIMIT, |limit| {
        usize::try_from(limit).map_or(SEARCH_CONTENTS_MAX, |limit| limit.min(SEARCH_CONTENTS_MAX))
    });
    let query = files::ContentQuery {
        query,
        limit,
        case_sensitive: payload.case_sensitive,
        whole_word: payload.whole_word,
        use_regex: payload.use_regex,
    };
    match files::search_contents(
        &state.reads,
        state.sandboxes.as_ref(),
        &viewer_scope(&identity),
        &cwd,
        &query,
    )
    .await
    {
        Ok(found) => vec![success_exit(&id, found)],
        Err(error) => vec![file_failure_exit(&id, tag, &error, &identity)],
    }
}

/// Never refused: every path the viewer may not see answers `null`.
pub(super) async fn filesystem_get_metadata(
    request: Request<FilesystemGetMetadataInput>,
) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let paths: Vec<String> = payload
        .paths
        .into_iter()
        .take(FILESYSTEM_METADATA_BATCH_LIMIT)
        .collect();
    let metadata = files::file_metadata(
        &state.reads,
        state.file_sources(),
        &viewer_scope(&identity),
        &paths,
    )
    .await;
    vec![success_exit(&id, metadata)]
}

pub(super) async fn projects_go_to_definition(
    request: Request<ProjectGoToDefinitionInput>,
) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let position = |position: Option<u64>| {
        position
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
    };
    let (Some(cwd), Some(relative_path), Some(line), Some(column)) = (
        workspace_cwd(payload.cwd.as_deref()),
        relative_path(payload.relative_path.as_deref()),
        position(payload.line),
        position(payload.column),
    ) else {
        return vec![file_failure_exit(
            &id,
            tag,
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    match files::go_to_definition(
        &state.reads,
        state.sandboxes.as_ref(),
        &viewer_scope(&identity),
        &cwd,
        relative_path,
        line,
        column,
    )
    .await
    {
        Ok(result) => vec![success_exit(&id, result)],
        Err(error) => vec![file_failure_exit(&id, tag, &error, &identity)],
    }
}

pub(super) async fn projects_read_file(request: Request<ProjectReadFileInput>) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let (Some(cwd), Some(relative_path)) = (
        workspace_cwd(payload.cwd.as_deref()),
        relative_path(payload.relative_path.as_deref()),
    ) else {
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

pub(super) async fn projects_search_entries(
    request: Request<ProjectSearchEntriesInput>,
) -> Vec<Value> {
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
    let query = payload.query.as_deref().map_or("", str::trim);
    let Some(cwd) = workspace_cwd(payload.cwd.as_deref()) else {
        return vec![file_failure_exit(
            &id,
            tag,
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    let limit = payload.limit.map_or(SEARCH_ENTRIES_LIMIT, |limit| {
        (limit as usize).min(SEARCH_ENTRIES_MAX)
    });
    match files::search_entries(
        &state.reads,
        state.file_sources(),
        &viewer_scope(&identity),
        &cwd,
        query,
        limit,
        payload.kind.as_deref(),
    )
    .await
    {
        Ok(entries) => vec![success_exit(&id, entries)],
        Err(error) => vec![file_failure_exit(&id, tag, &error, &identity)],
    }
}

pub(super) async fn projects_write_file(request: Request<ProjectWriteFileInput>) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    // Empty contents are a legal write — truncating a file is a thing a person
    // does — so only an absent `contents` is unaddressable.
    let (Some(cwd), Some(relative_path), Some(contents)) = (
        workspace_cwd(payload.cwd.as_deref()),
        relative_path(payload.relative_path.as_deref()),
        payload.contents.as_deref(),
    ) else {
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
///
/// The decode refuses a request without a `resource`: the typed error carries
/// the resource back, so it has no typed answer to give.
pub(super) async fn assets_create_url(request: Request<AssetCreateUrlInput>) -> Vec<Value> {
    let Request {
        id,
        payload: AssetCreateUrlInput { resource },
        ..
    } = request;
    match assets::create_url(&resource) {
        Ok(minted) => vec![success_exit(&id, minted)],
        Err(error) => {
            // Each names the resource the client asked for, never this side.
            rpc_metrics::refused();
            vec![typed_failure_exit(
                &id,
                json!({
                    "_tag": error.tag(),
                    "resource": resource
                }),
            )]
        }
    }
}

/// Mints the URL the composer uploads a file to before sending the turn that
/// names it; the URL carries its own credential, so nothing about the upload
/// reaches this socket except the id.
pub(super) async fn attachments_create_upload_url(
    request: Request<AttachmentCreateUploadUrlInput>,
) -> Vec<Value> {
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

pub(super) async fn attachments_delete(request: Request<AttachmentDeleteInput>) -> Vec<Value> {
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
/// ([`v2_commands::project_mutation`]), whose typed refusal a malformed `Value`
/// payload gets rather than a decode Die. Answers the updated `Project`.
pub(super) async fn projects_mutate(request: Request<Value>) -> Vec<Value> {
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
