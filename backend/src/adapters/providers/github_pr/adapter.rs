//! GitHub PR adapter type and composition root.

use std::sync::Arc;

use crate::adapters::{Adapter, AdapterInboundPresentation, AdapterKind, AdapterOutboundContent};
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::ProviderTokenAuthDao;
use crate::secret::service::SecretService;

use super::auth::GithubPrAuth;

/// GitHub PR adapter.
pub struct GithubPrAdapter {
    /// Global webhook secret from env (`GITHUB_CHANNEL_WEBHOOK_SECRET`).
    /// Used as a fallback when the org connection has no `webhook_secret` set.
    global_webhook_secret: Option<String>,
    /// Frontend base URL (`config.auth.frontend_url`). Used to render the
    /// `Moatless task:` footer link on outbound comments and to recover the
    /// authoring task id from that link on inbound echoes / context backfill.
    frontend_url: String,
    /// GitHub identity selection and credential materialization.
    auth: GithubPrAuth,
}

impl GithubPrAdapter {
    pub(crate) fn new(
        global_webhook_secret: Option<String>,
        global_bot_token: Option<String>,
        frontend_url: String,
        provider_tokens: Arc<dyn ProviderTokenDao>,
        provider_token_auth: Option<Arc<dyn ProviderTokenAuthDao>>,
        github_app_service: Option<Arc<GitHubAppService>>,
        github_apps: Arc<GitHubAppRegistry>,
    ) -> Self {
        Self {
            global_webhook_secret,
            frontend_url,
            auth: GithubPrAuth::new(
                global_bot_token,
                provider_tokens,
                provider_token_auth,
                github_app_service,
                github_apps,
            ),
        }
    }

    /// Wire the secret service used to resolve `secret_ref` provider tokens.
    #[must_use]
    pub(crate) fn with_secrets(mut self, secrets: Arc<SecretService>) -> Self {
        self.auth.set_secrets(secrets);
        self
    }

    /// Point every REST client this Adapter builds at a caller-supplied root.
    ///
    /// [`Self::new`] resolves the root from the process environment, which
    /// leaves a test nowhere to put a stub except a process-global — and a
    /// process-global is shared with every other test in the binary. This is
    /// GitHub's form of `SlackAdapter::with_api` / `TelegramAdapter::with_api`:
    /// the root is injected because the client cannot be, tokens being selected
    /// per call.
    #[cfg(test)]
    #[must_use]
    pub(super) fn with_api_base(mut self, base_url: impl Into<String>) -> Self {
        self.auth.set_api_base(base_url);
        self
    }

    pub(super) fn auth(&self) -> &GithubPrAuth {
        &self.auth
    }

    pub(super) fn global_webhook_secret(&self) -> Option<&str> {
        self.global_webhook_secret.as_deref()
    }

    pub(super) fn frontend_url(&self) -> &str {
        &self.frontend_url
    }
}

impl Adapter for GithubPrAdapter {
    fn adapter_kind(&self) -> AdapterKind {
        AdapterKind::GithubPr
    }

    fn provider_key(&self) -> &'static str {
        "github_pr"
    }

    fn connection_provider_key(&self) -> &'static str {
        "github"
    }
}

impl AdapterOutboundContent for GithubPrAdapter {
    /// GitHub allows up to 65536 chars per issue/PR comment. Leave headroom for
    /// the optional `Moatless task:` footer that `send_reply_for_binding`
    /// appends.
    fn max_message_length(&self) -> usize {
        65_000
    }
}

#[async_trait::async_trait]
impl AdapterInboundPresentation for GithubPrAdapter {
    fn reply_instructions(&self) -> Option<String> {
        Some(
            "## GitHub PR Agent Notes\n\
             - Always read the live PR state via the `moat gh` commands from the \
               initial message (`pr view`, `pr diff`, `api …/comments`) — the \
               description and comments evolve after task creation.\n\
             - `moat gh pr reply <pr_number> <comment_id> --body \"…\"` — post a \
               threaded reply to a specific review/inline comment by its numeric \
               id (shown as `comment id:` in delivered messages and in \
               `moat gh api …/pulls/<pr>/comments` output).\n\
             - `moat gh pr comment <pr_number> --body \"…\"` — post a top-level \
               conversation comment.\n\
             - `moat messages reply --message <message_id> --text \"…\"` — reply \
               to a delivered message; routes as a threaded reply when the \
               message came from an inline review comment, top-level otherwise.\n\
             - Task completion does NOT auto-post anything to GitHub — when you \
               finish, post your final summary explicitly via \
               `moat gh pr comment`.\n\
             - Markdown is supported in all comment bodies."
                .to_string(),
        )
    }

    fn external_id_aliases(&self, external_id: &str) -> Vec<String> {
        super::external_id::aliases(external_id)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::adapters::{
        Adapter, AdapterDestinationBuilder, AdapterInboundPresentation, AdapterOutboundContent,
    };
    use crate::auth::github::registry::GitHubAppRegistry;
    use crate::auth::provider_token_dao::ProviderTokenDao;
    use crate::db::DbPool;

    use super::GithubPrAdapter;

    #[tokio::test]
    async fn provider_keys_separate_channel_and_integration_storage() {
        let provider = provider();
        assert_eq!(provider.provider_key(), "github_pr");
        assert_eq!(provider.connection_provider_key(), "github");
        assert!(!provider.allow_default_connection_fallback());
        assert!(!provider.allow_wildcard_trigger_fallback());
    }

    #[tokio::test]
    async fn outbound_content_leaves_room_for_task_footer() {
        assert_eq!(provider().max_message_length(), 65_000);
    }

    #[tokio::test]
    async fn reply_instructions_cover_reply_paths_and_completion_behavior() {
        let notes = provider().reply_instructions().expect("instructions");
        assert!(notes.contains("## GitHub PR Agent Notes"));
        assert!(notes.contains("moat gh pr reply <pr_number> <comment_id>"));
        assert!(notes.contains("moat gh pr comment <pr_number>"));
        assert!(notes.contains("moat messages reply --message"));
        assert!(notes.contains("does NOT auto-post"));
    }

    #[tokio::test]
    async fn external_id_aliases_include_legacy_comment_ids_only() {
        let provider = provider();
        assert_eq!(
            provider.external_id_aliases("github:ic:42"),
            vec!["github:ic:42".to_string(), "42".to_string()]
        );
        assert_eq!(
            provider.external_id_aliases("github:rc:99"),
            vec!["github:rc:99".to_string(), "99".to_string()]
        );
        assert_eq!(
            provider.external_id_aliases("github:pr:opened:7"),
            vec!["github:pr:opened:7".to_string()]
        );
    }

    fn provider() -> GithubPrAdapter {
        GithubPrAdapter::new(
            None,
            None,
            "https://app.example.com".to_string(),
            fake_provider_tokens(),
            None,
            None,
            Arc::new(GitHubAppRegistry::default()),
        )
    }

    fn fake_db_pool() -> DbPool {
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://fake:fake@localhost:1/fake")
            .unwrap()
    }

    fn fake_provider_tokens() -> Arc<dyn ProviderTokenDao> {
        Arc::new(crate::auth::provider_token_dao::PgProviderTokenDao(
            fake_db_pool(),
        ))
    }
}
