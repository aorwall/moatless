//! What `create` refuses before it asks the server, and the uploads
//! `--context-file` turns into.

use crate::client::{ClientError, MoatClient};
use crate::commands::input::upload_files;

/// Refuses a create when no Workspace source is named: `--workspace`, a joined
/// sandbox, or a `--parent` to inherit from.
///
/// Not a clap `required_unless_present`: inside a sandbox the ambient task is
/// the parent, and no flag carries it.
pub(super) fn require_a_workspace_source(
    workspace_id: Option<&str>,
    parent_task_id: Option<&str>,
) -> Result<(), ClientError> {
    if workspace_id.is_none() && parent_task_id.is_none() {
        return Err(ClientError::Api {
            status: 400,
            body: "--workspace is required: pass one, --same-sandbox, or --parent to inherit"
                .to_string(),
        });
    }

    Ok(())
}

pub(super) fn validate_context_files_have_message(
    message: &Option<String>,
    context_files: &[String],
) -> Result<(), ClientError> {
    if message.is_none() && !context_files.is_empty() {
        return Err(ClientError::Api {
            status: 400,
            body: "--context-file requires --message so the file contexts have an initial message to attach to".to_string(),
        });
    }

    Ok(())
}

/// Upload each `--context-file` and return the file ids to attach.
///
/// The bytes travel, deliberately: a file id is what the new Task's sandbox
/// fetches over gRPC before its agent runs, and a path is not one — the child
/// task would report `File metadata not found` and receive nothing.
pub(super) async fn upload_context_files(
    client: &MoatClient,
    paths: &[String],
) -> Result<Vec<String>, ClientError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    upload_files(client, paths)
        .await
        .map_err(|e| ClientError::Other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parent_or_a_join_stands_in_for_the_otherwise_required_workspace() {
        assert!(require_a_workspace_source(None, None).is_err());
        // A join resolves to the joined sandbox's workspace, so it reaches the
        // check as one.
        require_a_workspace_source(Some("ws_1"), None).expect("named outright");
        require_a_workspace_source(None, Some("parent-1")).expect("inherited from the parent");
    }

    #[test]
    fn context_files_require_initial_message() {
        let err = validate_context_files_have_message(
            &None,
            &["crates/cli/src/commands/tasks.rs".to_string()],
        )
        .expect_err("context files without message should be rejected");

        match err {
            ClientError::Api { status, body } => {
                assert_eq!(status, 400);
                assert!(body.contains("--context-file requires --message"));
            }
            other => panic!("expected API validation error, got {other:?}"),
        }

        validate_context_files_have_message(
            &Some("read these files".to_string()),
            &["crates/cli/src/commands/tasks.rs".to_string()],
        )
        .expect("message owns context files");
    }
}
