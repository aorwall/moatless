//! The Workspace a thread started with no project runs in: T3's scratch
//! project, reported as `scratchWorkspaceRoot` and found by
//! `projects.ensureScratch`.

use crate::nominal_path;
use crate::workspace_listing::WorkspaceListing;
use moatless_types::access_control::ScopeContext;
use moatless_workspaces::workspace::models::{ListedWorkspace, WorkspaceKind};
use tracing::warn;

/// The viewer's scratch Workspace id, or `None` when they have none.
///
/// `configured` is [`crate::config::UiRpcConfig::scratch_workspace`]; see
/// [`choose`] for the rule.
pub(crate) async fn scratch_workspace(
    workspaces: &dyn WorkspaceListing,
    viewer: &ScopeContext,
    configured: Option<&str>,
) -> Option<String> {
    match workspaces.list_readable_including(viewer, &[]).await {
        Ok(listed) => choose(&listed, configured).map(|listed| listed.workspace.id.clone()),
        Err(error) => {
            warn!(%error, "Failed to list Workspaces for the T3 scratch project");
            None
        }
    }
}

/// The project root the client recognises the scratch project by.
pub(crate) fn scratch_root(workspace_id: &str) -> String {
    nominal_path::nominal_root(workspace_id)
}

/// The configured Workspace when the viewer may read it; otherwise, with none
/// configured, the viewer's one readable blank Workspace.
///
/// Several blank Workspaces and no configuration choose none, rather than one
/// by name: a thread would land in a Workspace nobody picked.
fn choose<'a>(
    listed: &'a [ListedWorkspace],
    configured: Option<&str>,
) -> Option<&'a ListedWorkspace> {
    let mut live = listed
        .iter()
        .filter(|listed| listed.readable && !listed.workspace.deleted);
    if let Some(configured) = configured {
        return live.find(|listed| listed.workspace.id == configured);
    }
    let mut blank = live.filter(|listed| listed.workspace.kind == WorkspaceKind::Blank);
    match (blank.next(), blank.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_workspaces::test_support::make_workspace_row;

    fn listed(id: &str, kind: WorkspaceKind) -> ListedWorkspace {
        let mut workspace = make_workspace_row(id, "user-1");
        workspace.kind = kind;
        ListedWorkspace {
            workspace,
            readable: true,
        }
    }

    fn chosen(listed: &[ListedWorkspace], configured: Option<&str>) -> Option<String> {
        choose(listed, configured).map(|listed| listed.workspace.id.clone())
    }

    #[test]
    fn the_only_blank_workspace_is_the_scratch_one() {
        let listed = [
            listed("repo", WorkspaceKind::Single),
            listed("blank", WorkspaceKind::Blank),
        ];
        assert_eq!(chosen(&listed, None).as_deref(), Some("blank"));
    }

    #[test]
    fn several_blank_workspaces_choose_none_unless_one_is_configured() {
        let listed = [
            listed("a", WorkspaceKind::Blank),
            listed("b", WorkspaceKind::Blank),
        ];
        assert_eq!(chosen(&listed, None), None);
        assert_eq!(chosen(&listed, Some("b")).as_deref(), Some("b"));
    }

    #[test]
    fn a_configured_workspace_the_viewer_cannot_read_is_no_scratch_project() {
        let mut referenced = listed("configured", WorkspaceKind::Blank);
        referenced.readable = false;
        let deleted = {
            let mut deleted = listed("gone", WorkspaceKind::Blank);
            deleted.workspace.deleted = true;
            deleted
        };
        let listed = [referenced, deleted, listed("other", WorkspaceKind::Blank)];
        assert_eq!(chosen(&listed, Some("configured")), None);
        assert_eq!(chosen(&listed, Some("gone")), None);
    }

    #[test]
    fn no_blank_workspace_is_no_scratch_project() {
        let listed = [listed("repo", WorkspaceKind::Single)];
        assert_eq!(chosen(&listed, None), None);
    }
}
