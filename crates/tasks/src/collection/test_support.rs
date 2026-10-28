//! A [`Records`] over a mocked store, for the collection and artifact tests.

use std::sync::Arc;

use moatless_workspace_ports::{MockRepositoryReader, MockWorkspaceResolver};

use super::dao::MockRecordDao;
use super::operations::Records;

/// `Records` over `store` whose Workspaces and repositories every caller reads.
pub(crate) fn records(store: MockRecordDao) -> Records {
    let mut repositories = MockRepositoryReader::new();
    repositories.expect_describe().returning(|_, id| {
        Ok(Some(
            moatless_workspaces::test_support::make_repository_row(id, "user-1"),
        ))
    });
    let mut workspaces = MockWorkspaceResolver::new();
    workspaces.expect_describe().returning(|_, id| {
        Ok(Some(moatless_workspaces::test_support::make_workspace_row(
            id, "user-1",
        )))
    });
    Records::new(
        Arc::new(store),
        Arc::new(repositories),
        Arc::new(workspaces),
    )
}
