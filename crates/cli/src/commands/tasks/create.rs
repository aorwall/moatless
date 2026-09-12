//! What `create` refuses before it asks the server, and the message
//! contexts `--context-file` turns into.

use crate::client::ClientError;
use moatless_api_schemas::task::{FileContextData, MessageContextInput};

/// Refuse a create the server would refuse: every Task runs in a Workspace, and
/// only `--workspace`, a joined sandbox, or a parent to inherit from names one.
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

pub(super) fn context_files_to_contexts(paths: &[String]) -> Vec<MessageContextInput> {
    paths
        .iter()
        .map(|path| {
            let reference_id = normalize_context_file_path(path);
            let path_ref = std::path::Path::new(path);
            let filename = path_ref
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(reference_id.as_str())
                .to_string();
            let mime_type = mime_guess::from_path(path_ref)
                .first_or_octet_stream()
                .to_string();
            let size = std::fs::metadata(path_ref)
                .map(|metadata| metadata.len().min(i64::MAX as u64) as i64)
                .unwrap_or_default();

            MessageContextInput::File {
                reference_id,
                data: FileContextData {
                    filename,
                    mime_type,
                    size,
                },
            }
        })
        .collect()
}

fn normalize_context_file_path(path: &str) -> String {
    let path_ref = std::path::Path::new(path);
    let relative = if path_ref.is_absolute() {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| path_ref.strip_prefix(cwd).ok())
            .unwrap_or(path_ref)
    } else {
        path_ref
    };

    let normalized = relative.to_string_lossy().replace('\\', "/");
    normalized
        .strip_prefix("./")
        .unwrap_or(normalized.as_str())
        .to_string()
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
    fn normalizes_relative_context_file_path() {
        assert_eq!(
            normalize_context_file_path("./crates/cli/src/commands/tasks.rs"),
            "crates/cli/src/commands/tasks.rs"
        );
    }

    #[test]
    fn context_files_become_file_contexts() {
        let contexts = context_files_to_contexts(&["CONTEXT.md".to_string()]);

        assert_eq!(contexts.len(), 1);
        match &contexts[0] {
            MessageContextInput::File { reference_id, data } => {
                assert_eq!(reference_id, "CONTEXT.md");
                assert_eq!(data.filename, "CONTEXT.md");
                assert_eq!(data.mime_type, "text/markdown");
            }
            _ => panic!("expected file context"),
        }
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
