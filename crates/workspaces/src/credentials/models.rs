/// Where a [`super::CredentialService::store`] call originated.
/// to skip the originating sandbox so a rotated value doesn't echo back as a
/// fresh `RefreshCredential` push.
#[derive(Debug, Clone)]
pub enum WriteSource {
    /// Sandbox-side rotation pushed via `BackendSandboxService::UpdateCredential`.
    SandboxRotation { task_id: String },
    /// Backend-driven refresh (e.g. GitHub App user-token expiry path).
    BackendRefresh,
    /// User clicked "save" / completed an OAuth flow in the settings UI.
    UserUiSave,
}

/// Which persist path [`super::CredentialService::store`] took. Returned so the
/// tasks-band `CredentialSync` can make the broadcast decision explicit at the
/// call site instead of hiding it inside the persist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOutcome {
    /// GitHub token routed to the encrypted secret store. Never broadcast —
    /// `should_broadcast("GITHUB_TOKEN")` is false.
    SecretRef,
    /// Dedup hit: the stored value already equalled `value`, nothing was
    /// persisted. Must not broadcast, or a no-op write echoes back.
    Unchanged,
    /// A new value was upserted. Broadcast iff `should_broadcast(key)`.
    Persisted,
}

/// Per-key broadcast policy. `pub(crate)` so the tasks-band `CredentialSync`
/// can gate its fan-out on it (the broadcast decision moved up with the
/// orchestration in 9c; the policy that keys are file-resident stays here).
pub fn should_broadcast(key: &str) -> bool {
    matches!(
        key,
        // File-resident credentials — sandboxes can't pull these on demand,
        // so the backend must push.
        "CODEX_AUTH_JSON" | "CLAUDE_CODE_OAUTH_TOKEN"
    )
}

/// Map a credential env-var key to the provider name in `provider_tokens`.
///
/// Returns `None` for keys that aren't backed by a provider row (e.g. raw
/// secrets that live elsewhere).
pub(crate) fn credential_key_to_provider(key: &str) -> Option<&'static str> {
    match key {
        "CODEX_AUTH_JSON" => Some("codex"),
        "OPENAI_API_KEY" => Some("openai"),
        "GITHUB_TOKEN" => Some("github"),
        "GITLAB_TOKEN" => Some("gitlab"),
        "GITNESS_TOKEN" => Some("gitness"),
        "CLAUDE_CODE_OAUTH_TOKEN" | "ANTHROPIC_API_KEY" => Some("claude"),
        _ => None,
    }
}
