//! GitHub PR adapter type and composition root.

use std::sync::Arc;

use moatless_adapter_model::capabilities::{
    Adapter, AdapterInboundPresentation, AdapterOutboundContent,
};
use moatless_adapter_model::connections::ConnectionLookup;
use moatless_adapter_model::delivery::DeliveryAdmission;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::loops::LoopReader;
use moatless_adapter_model::registry::Adapters;
use moatless_loops::bindings::operations::Bindings;
use moatless_ports::identity::GitHostTokenIssuer;
use moatless_ports::identity::ProviderIdentityLink;
use moatless_types::adapter::AdapterKind;

use super::auth::GithubPrAuth;

/// GitHub PR adapter.
pub struct GithubPrAdapter {
    /// Global webhook secret from env (`GITHUB_CHANNEL_WEBHOOK_SECRET`). Used
    /// as a fallback when the org connection has no `webhook_secret` set.
    global_webhook_secret: Option<String>,
    /// Frontend base URL (`config.auth.frontend_url`). Used to render the
    /// `Moatless task:` footer link on outbound comments and to recover the
    /// authoring task id from that link on inbound echoes / context backfill.
    frontend_url: String,
    /// GitHub identity selection and credential materialization.
    auth: GithubPrAuth,
    /// Persistence capabilities supplied by [`Self::with_ingress_ports`].
    adapter_connections: Option<Arc<dyn ConnectionLookup>>,
    loops: Option<Arc<dyn LoopReader>>,
    task_bindings: Option<Arc<Bindings>>,
    users: Option<Arc<dyn ProviderIdentityLink>>,
    adapter_delivery: Option<Arc<dyn DeliveryAdmission>>,
}

/// The internal error for an ingress port read on an adapter that never
/// received one — a composition-root bug, not a request fault.
fn missing_port(name: &str) -> AdapterError {
    SharedAdapterError::Internal(format!("GitHub PR adapter missing {name} port")).into()
}

impl GithubPrAdapter {
    /// Register `GithubPrAdapter`'s capabilities into `adapters`. See
    /// [`moatless_loops::adapters::Adapters`].
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
    /// Only the `AdapterDeliveryStore` behind this `DeliveryAdmission` carries
    /// policy state; the rest are stateless pool wrappers.
    #[must_use]
    pub fn with_ingress_ports(
        mut self,
        adapter_connections: Arc<dyn ConnectionLookup>,
        loops: Arc<dyn LoopReader>,
        task_bindings: Arc<Bindings>,
        users: Arc<dyn ProviderIdentityLink>,
        adapter_delivery: Arc<dyn DeliveryAdmission>,
    ) -> Self {
        self.adapter_connections = Some(adapter_connections);
        self.loops = Some(loops);
        self.task_bindings = Some(task_bindings);
        self.users = Some(users);
        self.adapter_delivery = Some(adapter_delivery);
        self
    }

    /// The connection store, for connection resolution during admission. `Err`
    /// only if this adapter was built without ingress ports (a test
    /// constructor).
    pub(super) fn require_adapter_connections(
        &self,
    ) -> Result<&dyn ConnectionLookup, AdapterError> {
        self.adapter_connections
            .as_deref()
            .ok_or_else(|| missing_port("adapter_connections"))
    }

    /// The Subscription reads, for subscription resolution during admission.
    pub(super) fn require_loops(&self) -> Result<&dyn LoopReader, AdapterError> {
        self.loops.as_deref().ok_or_else(|| missing_port("loops"))
    }

    /// The binding store, for the subscription/binding reads admission plans
    /// on.
    pub(super) fn require_task_bindings(&self) -> Result<&Arc<Bindings>, AdapterError> {
        self.task_bindings
            .as_ref()
            .ok_or_else(|| missing_port("task_bindings"))
    }

    /// Resolves an inbound GitHub handle to the system user it belongs to.
    pub(super) fn require_users(&self) -> Result<&dyn ProviderIdentityLink, AdapterError> {
        self.users.as_deref().ok_or_else(|| missing_port("users"))
    }

    /// The durable delivery seam admitted webhook work is handed to.
    pub(super) fn require_adapter_delivery(&self) -> Result<&dyn DeliveryAdmission, AdapterError> {
        self.adapter_delivery
            .as_deref()
            .ok_or_else(|| missing_port("adapter_delivery"))
    }

    /// Point every REST client this Adapter builds at a caller-supplied root
    /// (test-only).
    ///
    /// [`Self::new`] pulls the root from the process environment, which has no
    /// per-test injection point otherwise.
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
    async fn reply_instructions(&self) -> Option<String> {
        let in_reply = super::text::IN_REPLY_MARKER;
        let new_thread = super::text::NEW_THREAD_MARKER;
        Some(format!(
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
             - Markdown is supported in all comment bodies.\n\
             \n\
             ### When to respond\n\
             You receive every comment on the pull request, not only the ones \
             addressed to you. Decide on who a comment addresses, not on what \
             it is about. A comment is addressed to you when any of these \
             holds:\n\
             - It names you with an `@` mention. A mention outranks every rule \
               below. Read the account you post as with `moat gh api user`; \
               when that call fails you are posting as a bot, and `user.login` \
               on a comment you already posted names it.\n\
             - It replies inside a thread you started — the `{in_reply} <id>` \
               line names a comment you posted.\n\
             - You opened this pull request. Review feedback on your own diff \
               is addressed to you, whichever thread it opens.\n\
             \n\
             - **Stay silent** when an inline comment carries `{new_thread}` \
               and you did not open this pull request. It opens a conversation \
               with the pull request author, not with you.\n\
             - **Stay silent** when two other people are talking, even on a \
               line you reviewed. Reviewing a line does not make every later \
               comment on that line yours to answer.\n\
             - Only an inline review comment carries those two lines. A \
               top-level conversation comment carries neither, so judge it by \
               what it says: answer it when it asks you something or asks for \
               work only you are doing, and stay silent for a status update, \
               an acknowledgement, or a question aimed at somebody else.\n\
             - When in doubt, stay silent.\n\
             - To stay silent, do nothing. Post no comment and no reply. \
               Posting your own review findings is not a response and is \
               always allowed."
        ))
    }

    fn external_id_aliases(&self, external_id: &str) -> Vec<String> {
        super::external_id::aliases(external_id)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use moatless_adapter_model::capabilities::{
        Adapter, AdapterDestinationBuilder, AdapterInboundPresentation, AdapterOutboundContent,
    };
    use moatless_identity::auth::test_support::StubProviderTokens;

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
        let notes = provider().reply_instructions().await.expect("instructions");
        assert!(notes.contains("## GitHub PR Agent Notes"));
        assert!(notes.contains("moat gh pr reply <pr_number> <comment_id>"));
        assert!(notes.contains("moat gh pr comment <pr_number>"));
        assert!(notes.contains("moat messages reply --message"));
        assert!(notes.contains("does NOT auto-post"));
    }

    /// Asserted against the renderer's own constants, so a reworded marker
    /// cannot leave the instructions naming a line no message carries.
    #[tokio::test]
    async fn reply_instructions_state_when_a_comment_is_addressed_to_the_agent() {
        let notes = provider().reply_instructions().await.expect("instructions");
        assert!(notes.contains("### When to respond"));
        assert!(notes.contains(&format!("`{} <id>`", crate::text::IN_REPLY_MARKER)));
        assert!(notes.contains(crate::text::NEW_THREAD_MARKER));
        assert!(notes.contains("A mention outranks every rule below."));
        assert!(notes.contains("You opened this pull request."));
        assert!(notes.contains("top-level conversation comment carries neither"));
        assert!(notes.contains("To stay silent, do nothing."));
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
            moatless_identity::git_host_tokens::seam_over(Arc::new(StubProviderTokens::default())),
        )
    }
}
