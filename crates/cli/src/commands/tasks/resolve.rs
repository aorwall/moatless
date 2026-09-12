//! Turning what a caller typed into the ids the API takes: a short id, a
//! repository or workspace name, a sandbox to join.

use crate::client::{ClientError, MoatClient};
use moatless_api_schemas::task::ListTasksQuery;

/// True if `s` looks like a full `task_id` (UUID: 36 chars with 4 hyphens).
fn looks_like_task_id(s: &str) -> bool {
    s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4
}

pub(super) fn resolve_parent_task_id(
    ambient_task_id: Option<&str>,
    explicit_parent: Option<String>,
) -> Option<String> {
    ambient_task_id.map(String::from).or(explicit_parent)
}

/// Resolve a task reference to a full `task_id`.
///
/// Accepts either a full `task_id` (UUID — passed through) or the 8-char shortId
/// shown in the UI, which is looked up among the most recent tasks. Short ids
/// are unique, so the first match wins.
pub(super) async fn resolve_task_id(
    client: &MoatClient,
    task_ref: &str,
) -> Result<String, ClientError> {
    if looks_like_task_id(task_ref) {
        return Ok(task_ref.to_string());
    }
    let query = ListTasksQuery {
        limit: Some(200),
        offset: None,
        status: None,
        repository_id: None,
        created_by: None,
        tag: None,
        parent_task_id: None,
        followed_by: None,
    };
    let list = client.api().list_tasks(&query).await?;
    match list
        .items
        .iter()
        .find(|t| t.short_id.as_deref() == Some(task_ref))
    {
        Some(t) => Ok(t.task_id.clone()),
        None => Err(ClientError::Api {
            status: 404,
            body: format!(
                "No task found for '{task_ref}'. Pass a full task_id (UUID), or run 'moat tasks list' — short ids resolve only among the 200 most recent tasks."
            ),
        }),
    }
}

/// The Sandbox a `--same-sandbox` task will join, and the Workspace that
/// Sandbox already materializes.
pub(super) struct JoinTarget {
    pub(super) task: String,
    pub(super) sandbox: String,
    /// `None` only for a task whose workspace lookup came back empty; the
    /// server then resolves it the same way it would for a fresh task.
    pub(super) workspace: Option<String>,
}

/// Resolve `--same-sandbox` to the Sandbox to join.
///
/// The flag takes a *task* id because that is the identifier a caller has —
/// sandbox ids appear in no listing anyone reads. `task_ref` is `None` for a
/// bare `--same-sandbox`, meaning "the task I am running as", which only has an
/// answer inside a sandbox.
///
/// The workspace rides along: joining requires matching the Sandbox's
/// workspace, so making the caller name it again would only be a way to get it
/// wrong.
pub(super) async fn resolve_same_sandbox(
    client: &MoatClient,
    ambient_task_id: Option<&str>,
    task_ref: Option<&str>,
) -> Result<JoinTarget, ClientError> {
    let task_id = match task_ref {
        Some(task_ref) => resolve_task_id(client, task_ref).await?,
        None => ambient_task_id
            .map(String::from)
            .ok_or_else(|| ClientError::Api {
                status: 400,
                body: "--same-sandbox needs a task id when run outside a sandbox: TASK_ID is not set, so there is no current task whose sandbox to join.".to_string(),
            })?,
    };
    let task = client.api().get_task(&task_id).await?;
    let sandbox = task.sandbox_id.ok_or_else(|| ClientError::Api {
        status: 400,
        body: format!(
            "Task {task_id} has no sandbox to join — it has never been started. Start it with 'moat tasks start {task_id}', or omit --same-sandbox to give this task its own sandbox."
        ),
    })?;
    Ok(JoinTarget {
        task: task_id,
        sandbox,
        workspace: task.workspace_id,
    })
}

/// Resolve a repo argument to a repository ID.
/// If it looks like a UUID, pass through. Otherwise, search by name.
pub(super) async fn resolve_repo_id(
    client: &MoatClient,
    repo_arg: &str,
) -> Result<String, ClientError> {
    // Simple UUID check: 36 chars with 4 hyphens.
    if repo_arg.len() == 36 && repo_arg.chars().filter(|c| *c == '-').count() == 4 {
        return Ok(repo_arg.to_string());
    }

    let repos = client.api().list_repositories().await?;

    let matches: Vec<_> = repos
        .iter()
        .filter(|r| r.name.eq_ignore_ascii_case(repo_arg))
        .collect();

    match matches.len() {
        1 => Ok(matches[0].id.clone()),
        0 => Err(ClientError::Api {
            status: 404,
            body: format!(
                "No repository found with name '{repo_arg}'. Run 'moat repos list' to see available repositories."
            ),
        }),
        _ => {
            let names: Vec<_> = matches
                .iter()
                .map(|r| format!("{} ({})", r.name, r.id))
                .collect();
            Err(ClientError::Api {
                status: 400,
                body: format!(
                    "Multiple repositories match '{repo_arg}': {}",
                    names.join(", ")
                ),
            })
        }
    }
}

/// Resolve a workspace argument to a workspace ID.
/// `ws_`-prefixed IDs pass through. Otherwise, search by name.
pub(super) async fn resolve_workspace_id(
    client: &MoatClient,
    ws_arg: &str,
) -> Result<String, ClientError> {
    if ws_arg.starts_with("ws_") {
        return Ok(ws_arg.to_string());
    }

    let workspaces = client.api().list_workspaces().await?;

    let matches: Vec<_> = workspaces
        .iter()
        .filter(|w| w.name.eq_ignore_ascii_case(ws_arg))
        .collect();

    match matches.len() {
        1 => Ok(matches[0].id.clone()),
        0 => Err(ClientError::Api {
            status: 404,
            body: format!(
                "No workspace found with name '{ws_arg}'. Run 'moat workspaces list' to see available workspaces."
            ),
        }),
        _ => {
            let names: Vec<_> = matches
                .iter()
                .map(|w| format!("{} ({})", w.name, w.id))
                .collect();
            Err(ClientError::Api {
                status: 400,
                body: format!("Multiple workspaces match '{ws_arg}': {}", names.join(", ")),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_full_uuid_task_id() {
        assert!(looks_like_task_id("1f0d0ebb-0a8e-49c2-94ff-77e8ea50b8bb"));
        assert!(looks_like_task_id("873cc644-04af-415d-9e15-fe167f4d15dd"));
    }

    #[test]
    fn rejects_short_id_and_junk() {
        // 8-char shortIds shown in the UI must NOT be treated as full ids.
        assert!(!looks_like_task_id("K9PCBWwH"));
        assert!(!looks_like_task_id("Gs0d_Tcb"));
        assert!(!looks_like_task_id(""));
        // Right length, wrong hyphen count.
        assert!(!looks_like_task_id(
            "1f0d0ebb00a8e049c2094ff077e8ea50b8bbxx"
        ));
    }

    #[test]
    fn ambient_sandbox_task_is_always_the_parent() {
        assert_eq!(
            resolve_parent_task_id(Some("ambient-task"), Some("other-task".to_string())),
            Some("ambient-task".to_string())
        );
        assert_eq!(
            resolve_parent_task_id(None, Some("explicit-task".to_string())),
            Some("explicit-task".to_string())
        );
    }
}
