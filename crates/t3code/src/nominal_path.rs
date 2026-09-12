//! The nominal paths this crate issues, and the inverses that read them back.
//!
//! Every path here is synthesized rather than observed, and the client sends it
//! back as a `cwd`, so each one is declared beside the function that reverses it.

/// Prefix of the nominal root path reported for each workspace.
///
/// T3 treats a project's root path as an identity, and refuses a duplicate on
/// the write path. Moatless checks every workspace out under the same
/// per-sandbox directory, so reporting the real one would collapse every
/// project into one sidebar group; a distinct path is synthesized per workspace
/// instead. It is nominal either way, because the real checkout exists only
/// once a task has a sandbox.
const NOMINAL_ROOT_PREFIX: &str = "/moatless/workspaces";

/// Prefix of the nominal worktree path reported for each conversation.
///
/// The client sends a thread's `worktreePath` back as the `cwd` of every
/// workspace-file request it makes for that thread (`ChatView.tsx`:
/// `activeThread.worktreePath ?? activeProject.workspaceRoot`), so the image's
/// own `WORKSPACE_DIR` — one string for every conversation — would name every
/// conversation's files at once. A distinct path is synthesized per Task
/// instead, and [`files`](super::files) reverses it.
///
/// Its own namespace rather than under [`NOMINAL_ROOT_PREFIX`]: a Sandbox
/// checkout is not a directory inside its Workspace.
const NOMINAL_WORKTREE_PREFIX: &str = "/moatless/tasks";

/// A distinct, stable, nominal worktree path for one conversation.
pub(crate) fn nominal_worktree(task_id: &str) -> String {
    format!("{NOMINAL_WORKTREE_PREFIX}/{task_id}")
}

/// The conversation whose files a `cwd` names, if it names one.
///
/// The exact inverse of [`nominal_worktree`]. A `cwd` with a further path
/// segment is refused rather than truncated: the contract has no notion of
/// opening a subdirectory as the workspace root, so one is a path this module
/// never issued.
pub(crate) fn task_id_from_cwd(cwd: &str) -> Option<&str> {
    cwd.strip_prefix(NOMINAL_WORKTREE_PREFIX)?
        .strip_prefix('/')
        .filter(|task_id| !task_id.is_empty() && !task_id.contains('/'))
}

/// Whether a `cwd` is a project's root rather than a conversation's worktree.
///
/// The client falls back to the project root for a thread reporting no
/// worktree, so this is what a request for a conversation with no Sandbox
/// arrives as. Worth telling apart from an unrecognized path, which is a bug.
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
    /// to the workspace it names. A worktree path is a different namespace and
    /// is not read as a project root.
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
}
