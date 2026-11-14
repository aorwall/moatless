//! The nominal paths this crate issues, and the inverses that read them back.
//!
//! Every path here is synthesized rather than observed, and the client sends it
//! back as a `cwd`, so each one is declared beside the function that reverses
//! it.

/// Prefix of the nominal root path reported for each workspace.
///
/// T3 treats a project's root path as an identity and refuses a duplicate, so
/// each workspace gets a distinct synthesized path rather than its real
/// per-sandbox checkout dir — which would collapse every project into one
/// sidebar group, and doesn't exist before a task has a sandbox anyway.
const NOMINAL_ROOT_PREFIX: &str = "/moatless/workspaces";

/// Prefix of the nominal worktree path reported for each conversation.
///
/// The client echoes `worktreePath` back as `cwd` on every file request, so
/// `WORKSPACE_DIR` would collide across conversations; this is synthesized
/// per Task and reversed by [`files`](super::files) — its own namespace, not
/// under [`NOMINAL_ROOT_PREFIX`], since a checkout isn't a Workspace directory.
const NOMINAL_WORKTREE_PREFIX: &str = "/moatless/tasks";

/// A distinct, stable, nominal worktree path for one conversation.
pub(crate) fn nominal_worktree(task_id: &str) -> String {
    format!("{NOMINAL_WORKTREE_PREFIX}/{task_id}")
}

/// The conversation whose files a `cwd` names, if it names one.
///
/// The exact inverse of [`nominal_worktree`]. A `cwd` with a further path
/// segment is refused rather than truncated, since the contract has no notion
/// of opening a subdirectory as the workspace root.
pub(crate) fn task_id_from_cwd(cwd: &str) -> Option<&str> {
    cwd.strip_prefix(NOMINAL_WORKTREE_PREFIX)?
        .strip_prefix('/')
        .filter(|task_id| !task_id.is_empty() && !task_id.contains('/'))
}

/// The conversation an absolute path under its nominal worktree belongs to,
/// and the path relative to that worktree (empty for the worktree itself).
pub(crate) fn split_worktree_path(path: &str) -> Option<(&str, &str)> {
    let rest = path
        .strip_prefix(NOMINAL_WORKTREE_PREFIX)?
        .strip_prefix('/')?;
    let (task_id, relative) = rest.split_once('/').unwrap_or((rest, ""));
    (!task_id.is_empty()).then_some((task_id, relative))
}

/// Whether a `cwd` is a project's root rather than a conversation's worktree.
///
/// The client falls back to the project root for a thread reporting no
/// worktree, so this is what a Sandbox-less conversation's request arrives as —
/// worth telling apart from an unrecognized path, which is a bug.
pub(crate) fn is_project_root(cwd: &str) -> bool {
    workspace_id_from_project_root(cwd).is_some()
}

/// The workspace a project-root `cwd` names, if it names one.
///
/// The inverse of [`nominal_root`]: listing a not-yet-created thread's branches
/// reads the workspace id straight back out of the `cwd` the client returned.
pub(crate) fn workspace_id_from_project_root(cwd: &str) -> Option<&str> {
    cwd.strip_prefix(NOMINAL_ROOT_PREFIX)?
        .strip_prefix('/')
        .filter(|id| !id.is_empty() && !id.contains('/'))
}

/// A distinct, stable, nominal root path for one workspace.
pub(crate) fn nominal_root(workspace_id: &str) -> String {
    format!("{NOMINAL_ROOT_PREFIX}/{workspace_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The project root the shell reports and the workspace id read back out of
    /// it are each other's inverse, so a draft thread's branch request resolves
    /// to the workspace it names.
    ///
    /// A worktree path is a different namespace and is not read as a project
    /// root.
    #[test]
    fn a_project_root_round_trips_to_its_workspace_id() {
        assert_eq!(
            workspace_id_from_project_root(&nominal_root("ws-1")),
            Some("ws-1")
        );
        assert!(is_project_root(&nominal_root("ws-1")));
        assert_eq!(
            workspace_id_from_project_root(&nominal_worktree("task-1")),
            None
        );
        assert_eq!(
            workspace_id_from_project_root("/moatless/workspaces/ws/1"),
            None
        );
        assert_eq!(
            workspace_id_from_project_root("/moatless/workspaces/"),
            None
        );
    }

    /// A file chip names an absolute path under a thread's worktree, which is
    /// read back as the Task and the path inside it.
    #[test]
    fn a_path_under_a_worktree_splits_into_its_task_and_relative_path() {
        assert_eq!(
            split_worktree_path("/moatless/tasks/task-1/repo/src/a.rs"),
            Some(("task-1", "repo/src/a.rs"))
        );
        assert_eq!(
            split_worktree_path(&nominal_worktree("task-1")),
            Some(("task-1", ""))
        );
        assert_eq!(split_worktree_path("/moatless/tasks/"), None);
        assert_eq!(split_worktree_path("/home/me/a.rs"), None);
    }
}
