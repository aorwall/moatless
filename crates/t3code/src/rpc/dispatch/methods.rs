//! The table every connection answers from.

use crate::rpc::table::MethodTable;
use std::sync::LazyLock;

static METHODS: LazyLock<MethodTable> = LazyLock::new(|| {
    MethodTable::new()
        .merge(super::server::server_methods())
        .merge(super::artifacts::artifact_methods())
        .merge(super::orchestration::orchestration_methods())
        .merge(super::orchestration_v2::orchestration_v2_methods())
        .merge(super::streams::stream_methods())
        .merge(super::sandbox::sandbox_methods())
        .merge(super::projects::project_methods())
        .merge(super::preview::preview_methods())
        .merge(super::terminal::terminal_methods())
        .merge(super::vcs::vcs_methods())
});

/// Every method this surface answers or the contract declares.
pub(crate) fn methods() -> &'static MethodTable {
    &METHODS
}
