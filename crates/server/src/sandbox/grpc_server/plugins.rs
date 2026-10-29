//! The plugin archives a sandbox unpacks in place of cloning its plugins.

use std::pin::Pin;

use futures::{Stream, StreamExt};
use tonic::{Request, Response, Status};
use tracing::warn;

use moatless_workspaces::plugin::archive::PluginArchive;
use moatless_workspaces::repository::error::RepositoryError;
use moatless_workspaces::skill::error::SkillError;
use sandbox_proto::{FetchPluginRequest, PluginChunk};

use super::state::BackendSandboxServiceImpl;
use crate::sandbox::secrets::{resolve_task_actor, resolve_task_credential_targets};

/// The chunk stream [`fetch_plugin`] answers with.
pub(super) type PluginChunkStream = Pin<Box<dyn Stream<Item = Result<PluginChunk, Status>> + Send>>;

/// Split an archive into the chunks `FetchPlugin` streams, the version on the
/// first; never empty, so the version always arrives.
fn plugin_chunks(archive: &PluginArchive) -> Vec<PluginChunk> {
    const CHUNK_SIZE: usize = 1024 * 1024;

    let mut chunks: Vec<PluginChunk> = archive
        .data
        .chunks(CHUNK_SIZE)
        .map(|c| PluginChunk {
            data: c.to_vec(),
            ..PluginChunk::default()
        })
        .collect();
    if chunks.is_empty() {
        chunks.push(PluginChunk::default());
    }
    chunks[0].version = archive.version.clone();
    chunks
}

fn archive_status(e: &SkillError) -> Status {
    match e {
        SkillError::AuthRequired { .. } => Status::permission_denied(e.to_string()),
        SkillError::UnsupportedProvider { .. } => Status::failed_precondition(e.to_string()),
        SkillError::Fetch { .. } => Status::unavailable(e.to_string()),
        SkillError::RepositoryToken(RepositoryError::Store(_)) => {
            Status::internal("Failed to fetch plugin")
        }
        SkillError::RepositoryToken(_) => {
            Status::failed_precondition(moatless_core::error::causal_chain(e))
        }
        _ => Status::internal("Failed to fetch plugin"),
    }
}

/// Serve one of the task's plugins, looked up in the plugin list auto-provision
/// resolved for the same run-as identity, so a sandbox can fetch only what it
/// was handed.
pub(super) async fn fetch_plugin(
    service: &BackendSandboxServiceImpl,
    request: Request<FetchPluginRequest>,
) -> Result<Response<PluginChunkStream>, Status> {
    let task_id = request.get_ref().task_id.clone();
    let grant = service.authorizer.authorize(&request, &task_id).await?;
    let req = request.into_inner();
    let Some(handlers) = service.handlers.as_ref() else {
        return Err(Status::failed_precondition("Plugins need a database"));
    };

    let targets = resolve_task_credential_targets(Some(&handlers.sandboxes), &task_id).await;
    let Some(workspace_id) = targets.workspace_id.as_deref() else {
        return Err(Status::not_found("Task has no workspace"));
    };
    // Parsed as `auto_provision` parses it: a non-UUID run-as is nobody.
    let run_as = targets
        .run_as_user_id
        .as_deref()
        .and_then(|id| uuid::Uuid::parse_str(id).ok());
    let deliveries = handlers
        .workspace_resolver
        .plugin_deliveries(run_as, workspace_id)
        .await
        .map_err(|e| {
            warn!(task_id, error = %e, "Failed to resolve the task's plugins");
            Status::internal("Failed to resolve plugins")
        })?;
    let Some(plugin) = deliveries
        .picked
        .into_iter()
        .find(|p| p.name == req.plugin_name)
    else {
        return Err(Status::not_found(format!(
            "Plugin {} is not picked for this task",
            req.plugin_name
        )));
    };

    let actor = resolve_task_actor(
        Some(&handlers.workspace_resolver),
        Some(workspace_id),
        targets.run_as_user_id.as_deref(),
        &grant.ctx.user_id,
    )
    .await
    // A Workspace admin picked this Plugin, so the owner's own access does not
    // cap it: capping would withhold a picked Plugin from an owner who cannot
    // read its repository.
    .on_behalf_of(None);
    let known_version = Some(req.known_version.as_str()).filter(|v| !v.is_empty());
    let archive = handlers
        .plugin_archives
        .archive(&actor, &plugin.git_url, known_version)
        .await
        .map_err(|e| {
            warn!(task_id, plugin = %plugin.name, error = %e, "Failed to fetch plugin archive");
            archive_status(&e)
        })?;

    let chunks = match archive {
        Some(archive) => plugin_chunks(&archive),
        None => vec![PluginChunk {
            data: Vec::new(),
            version: req.known_version,
            unchanged: true,
        }],
    };
    let stream = futures::stream::iter(chunks).map(Ok::<PluginChunk, Status>);
    Ok(Response::new(Box::pin(stream)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn the_version_rides_the_first_chunk_only() {
        let archive = PluginArchive {
            version: "sha-1".to_string(),
            data: Arc::new(vec![7; 1024 * 1024 + 1]),
        };

        let chunks = plugin_chunks(&archive);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].version, "sha-1");
        assert!(chunks[1].version.is_empty());
        assert_eq!(
            chunks.iter().map(|c| c.data.len()).sum::<usize>(),
            1024 * 1024 + 1
        );
    }

    #[test]
    fn an_empty_archive_still_names_its_version() {
        let archive = PluginArchive {
            version: "sha-1".to_string(),
            data: Arc::new(Vec::new()),
        };

        let chunks = plugin_chunks(&archive);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].version, "sha-1");
    }

    #[test]
    fn only_a_credential_the_user_can_fix_reaches_the_sandbox_with_its_cause() {
        let broken = SkillError::RepositoryToken(RepositoryError::ProviderError(
            "the GitHub App authorization must be reconnected".to_string(),
        ));
        let store = SkillError::RepositoryToken(RepositoryError::Store(
            moatless_types::store::StoreError::new("read provider_tokens"),
        ));

        let broken = archive_status(&broken);
        let store = archive_status(&store);

        assert_eq!(broken.code(), tonic::Code::FailedPrecondition);
        assert!(broken.message().contains("must be reconnected"));
        assert_eq!(store.code(), tonic::Code::Internal);
        assert!(!store.message().contains("provider_tokens"));
    }
}
