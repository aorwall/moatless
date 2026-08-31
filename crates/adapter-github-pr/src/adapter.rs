//! GitHub PR adapter type and composition root.

use std::sync::Arc;

use moatless_backend_core::identity::GitHostTokenIssuer;
use moatless_adapter_model::dao::AdapterConnectionDao;
use moatless_loops::adapters::delivery::AdapterDeliveryStore;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_types::adapter::AdapterKind;
use moatless_adapter_model::capabilities::{Adapter, AdapterInboundPresentation, AdapterOutboundContent};
use moatless_loops::adapters::Adapters;
use moatless_loops::loops::ports::LoopReader;
use moatless_backend_core::identity::ProviderIdentityLink;
use moatless_loops::bindings::dao::TaskBindingDao;

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
    /// Binding ports the adapter's own routes read through this adapter rather
    /// than an `AppState` — the webhook ingress path
    /// ([`super::ingress::receive`]) and the typed-API bind path
    /// ([`super::api_routes`], via [`Self::require_binding_ports`]). They are
    /// present on the composition-root instance (see [`Self::with_ingress_ports`])
    /// and absent on the `#[cfg(test)]` constructors, which reach neither.
    adapter_connections: Option<Arc<dyn AdapterConnectionDao>>,
    loops: Option<Arc<dyn LoopReader>>,
    task_bindings: Option<Arc<dyn TaskBindingDao>>,
    users: Option<Arc<dyn ProviderIdentityLink>>,
    adapter_delivery: Option<Arc<AdapterDeliveryStore>>,
}

/// The internal error for an ingress port read on an adapter that never
/// received one — a composition-root bug, not a request fault.
fn missing_port(name: &str) -> AdapterError {
    SharedAdapterError::Internal(format!("GitHub PR adapter missing {name} port")).into()
}

/// The three owned handles [`GithubPrAdapter::require_binding_ports`] hands the
/// typed-API bind path for a [`moatless_loops::loops::firing::listen::Connector`] —
/// the connection store, the Subscription reads, and the binding store. A
/// transient return shape destructured at the call site, not a stored bundle.
type BindingPorts = (
    Arc<dyn AdapterConnectionDao>,
    Arc<dyn LoopReader>,
    Arc<dyn TaskBindingDao>,
);

impl GithubPrAdapter {
    /// Register this adapter's capabilities into `adapters`.
    ///
    /// Construction lives at the composition root; the provider owns the
    /// mapping from itself to the capability surfaces it implements.
    pub fn register_into(self: Arc<Self>, adapters: &mut Adapters) {
        let kind = AdapterKind::GithubPr;
        adapters.register_trait_object(self.clone());
        adapters.register_binding_reply(self.clone());
        adapters.register_message_reply(self.clone());
        adapters.register_reactions(self.clone());
        adapters.register_inbound_presentation(kind, self.clone());
        adapters.register_binding_status(kind, self);
    }

    pub fn new(
        global_webhook_secret: Option<String>,
        frontend_url: String,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    ) -> Self {
        Self {
            global_webhook_secret,
            frontend_url,
            auth: GithubPrAuth::new(git_host_tokens),
            adapter_connections: None,
            loops: None,
            task_bindings: None,
            users: None,
            adapter_delivery: None,
        }
    }

    /// Attach the ingress ports the webhook route reaches through this adapter,
    /// so [`super::handlers::github_webhook`] takes no `AppState`.
    ///
    /// Built by the composition root from the pool it already holds — the same
    /// local-construction move Telegram's `with_ingress_ports` and
    /// `LinearAdapter::with_tasks` make. All five are pool-wrapper DAOs the
    /// backend guidelines allow constructing locally precisely because they
    /// stay stateless, so a per-provider instance is
    /// interchangeable with the one `AppState` holds (`AdapterDeliveryStore`
    /// carries the one non-obvious case — policy state — and pins the invariant
    /// at its own definition).
    #[must_use]
    pub fn with_ingress_ports(
        mut self,
        adapter_connections: Arc<dyn AdapterConnectionDao>,
        loops: Arc<dyn LoopReader>,
        task_bindings: Arc<dyn TaskBindingDao>,
        users: Arc<dyn ProviderIdentityLink>,
        adapter_delivery: Arc<AdapterDeliveryStore>,
    ) -> Self {
        self.adapter_connections = Some(adapter_connections);
        self.loops = Some(loops);
        self.task_bindings = Some(task_bindings);
        self.users = Some(users);
        self.adapter_delivery = Some(adapter_delivery);
        self
    }

    /// The connection store, for connection resolution during admission. `Err`
    /// only if this adapter was built without ingress ports (a test constructor).
    pub(super) fn require_adapter_connections(
        &self,
    ) -> Result<&dyn AdapterConnectionDao, AdapterError> {
        self.adapter_connections
            .as_deref()
            .ok_or_else(|| missing_port("adapter_connections"))
    }

    /// The Subscription reads, for subscription resolution during admission.
    pub(super) fn require_loops(&self) -> Result<&dyn LoopReader, AdapterError> {
        self.loops.as_deref().ok_or_else(|| missing_port("loops"))
    }

    /// The binding store, for the subscription/binding reads admission plans on.
    pub(super) fn require_task_bindings(&self) -> Result<&Arc<dyn TaskBindingDao>, AdapterError> {
        self.task_bindings
            .as_ref()
            .ok_or_else(|| missing_port("task_bindings"))
    }

    /// Resolves an inbound GitHub handle to the system user it belongs to.
    pub(super) fn require_users(&self) -> Result<&dyn ProviderIdentityLink, AdapterError> {
        self.users
            .as_deref()
            .ok_or_else(|| missing_port("users"))
    }

    /// The durable delivery store admitted webhook work is handed to.
    pub(super) fn require_adapter_delivery(&self) -> Result<&AdapterDeliveryStore, AdapterError> {
        self.adapter_delivery
            .as_deref()
            .ok_or_else(|| missing_port("adapter_delivery"))
    }

    /// The three ports the typed-API bind path hands to a
    /// [`moatless_loops::loops::firing::listen::Connector`], as owned `Arc`s — the
    /// connection store, the Subscription reads, and the binding store. `Connector::new`
    /// takes them by value, so this clones rather than borrowing (unlike the
    /// ingress accessors above, which lend `receive` its borrows). `Err` only if
    /// this adapter was built without ingress ports (a test constructor).
    pub(super) fn require_binding_ports(&self) -> Result<BindingPorts, AdapterError> {
        Ok((
            self.adapter_connections
                .clone()
                .ok_or_else(|| missing_port("adapter_connections"))?,
            self.loops.clone().ok_or_else(|| missing_port("loops"))?,
            self.task_bindings
                .clone()
                .ok_or_else(|| missing_port("task_bindings"))?,
        ))
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

    use moatless_core::db::DbPool;
    use moatless_identity::auth::provider_token_dao::ProviderTokenDao;
    use moatless_adapter_model::capabilities::{Adapter, AdapterDestinationBuilder, AdapterInboundPresentation, AdapterOutboundContent};

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
            "https://app.example.com".to_string(),
            moatless_identity::git_host_tokens::seam_over(fake_provider_tokens()),
        )
    }

    fn fake_db_pool() -> DbPool {
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://fake:fake@localhost:1/fake")
            .unwrap()
    }

    fn fake_provider_tokens() -> Arc<dyn ProviderTokenDao> {
        Arc::new(moatless_identity::auth::provider_token_dao::PgProviderTokenDao(fake_db_pool()))
    }
}
