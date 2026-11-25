//! The table every connection answers from.

use crate::oauth::Scope;
use crate::rpc::table::MethodTable;
use std::sync::LazyLock;

/// Contract methods this surface does not answer: see [`MethodTable::declared`].
/// The client still calls `vcs.pull`, `vcs.createWorktree` and `vcs.init`.
/// Do not drop a row: an unknown tag is scope-checked as `orchestration:read`,
/// not its own scope.
const DECLARED: &[(&str, Scope)] = &[
    ("cloud.installRelayClient", Scope::RelayWrite),
    ("issueTrackers.bindProject", Scope::OrchestrationOperate),
    ("issueTrackers.connect", Scope::OrchestrationOperate),
    ("issues.react", Scope::OrchestrationOperate),
    ("preview.automationConnect", Scope::OrchestrationOperate),
    ("preview.automationFocusHost", Scope::OrchestrationOperate),
    ("preview.automationRespond", Scope::OrchestrationOperate),
    ("projects.add", Scope::OrchestrationOperate),
    ("projects.remove", Scope::OrchestrationOperate),
    (
        "pullRequests.generateReviewGuide",
        Scope::OrchestrationOperate,
    ),
    ("pullRequests.reviewGuide", Scope::OrchestrationRead),
    ("server.getSettings", Scope::OrchestrationRead),
    ("server.removeKeybinding", Scope::OrchestrationOperate),
    ("server.reportHostPowerState", Scope::OrchestrationOperate),
    ("server.retryResourceTelemetry", Scope::OrchestrationOperate),
    ("server.signalProcess", Scope::OrchestrationOperate),
    ("server.updateProvider", Scope::OrchestrationOperate),
    ("server.updateServer", Scope::OrchestrationOperate),
    (
        "server.updateServerWithProgress",
        Scope::OrchestrationOperate,
    ),
    ("server.upsertKeybinding", Scope::OrchestrationOperate),
    ("shell.openInEditor", Scope::OrchestrationOperate),
    ("sourceControl.cloneRepository", Scope::OrchestrationOperate),
    (
        "sourceControl.publishRepository",
        Scope::OrchestrationOperate,
    ),
    ("vcs.createRef", Scope::OrchestrationOperate),
    ("vcs.createWorktree", Scope::OrchestrationOperate),
    ("vcs.init", Scope::OrchestrationOperate),
    ("vcs.pull", Scope::OrchestrationOperate),
    ("vcs.removeWorktree", Scope::OrchestrationOperate),
];

static METHODS: LazyLock<MethodTable> = LazyLock::new(|| {
    let declared = DECLARED
        .iter()
        .fold(MethodTable::new(), |table, &(tag, scope)| {
            table.declared(tag, scope)
        });
    MethodTable::new()
        .merge(declared)
        .merge(super::server::server_methods())
        .merge(super::artifacts::artifact_methods())
        .merge(super::secrets::secret_methods())
        .merge(super::orchestration::orchestration_methods())
        .merge(super::orchestration_v2::orchestration_v2_methods())
        .merge(super::thread_reads::thread_read_methods())
        .merge(super::streams::stream_methods())
        .merge(super::sandbox::sandbox_methods())
        .merge(super::projects::project_methods())
        .merge(super::preview::preview_methods())
        .merge(super::terminal::terminal_methods())
        .merge(super::vcs::vcs_methods())
        .merge(super::issues::issue_methods())
});

/// Every method this surface answers or the contract declares.
pub(crate) fn methods() -> &'static MethodTable {
    &METHODS
}
