use moatless_types::access_control::ScopeContext;
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use moatless_types::ui_rpc::UserRole;

/// The access scope the session's viewer reads Moatless rows with.
pub(crate) fn viewer_scope(identity: &UiRpcAuthIdentity) -> ScopeContext {
    ScopeContext::new(
        identity.user_id.to_string(),
        matches!(identity.role, UserRole::Admin),
    )
}
