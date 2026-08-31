//! The Slack Adapter itself: its construction state, and the two capabilities
//! that are properties of the Adapter rather than behavior of a workflow.

use std::sync::Arc;

use tokio::sync::OnceCell;

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_types::adapter::AdapterKind;
use moatless_adapter_model::capabilities::{Adapter, AdapterOutboundContent};
use moatless_loops::adapters::Adapters;
use moatless_adapter_model::apps::{AdapterAppReader, AdapterAppSecrets};
use moatless_backend_core::identity::ProviderIdentityLink;
use moatless_core::config::Config;

use super::api::SlackApi;

/// One Slack app, in the shape this Adapter works in. The stored app is a bag
/// of secrets and settings; what Slack needs out of it is a bot client, the
/// secret that signs its webhooks, and the id it must not answer itself as.
pub(super) struct ResolvedSlackApp {
    pub(super) api: SlackApi,
    pub(super) signing_secret: String,
    pub(super) bot_user_id: Option<String>,
}

impl TryFrom<AdapterAppSecrets> for ResolvedSlackApp {
    type Error = AdapterError;

    fn try_from(app: AdapterAppSecrets) -> Result<Self, Self::Error> {
        let (Some(bot_token), Some(signing_secret)) =
            (app.secret("bot_token"), app.secret("signing_secret"))
        else {
            return Err(SharedAdapterError::AdapterApi(format!(
                "Slack Adapter app '{}' is missing required secrets",
                app.app_key
            ))
            .into());
        };
        Ok(Self {
            api: SlackApi::new(&bot_token),
            signing_secret,
            bot_user_id: app.setting("bot_user_id"),
        })
    }
}

/// Slack adapter.
pub struct SlackAdapter {
    api: SlackApi,
    signing_secret: String,
    bot_user_id: Option<String>,
    /// The stored Slack apps, when this Adapter was built over a database.
    /// `None` leaves every read on the three fields above.
    apps: Option<Arc<dyn AdapterAppReader>>,
    /// The Slack ↔ Moatless links, for the two paths the webhook's own
    /// `ProviderIdentityLink` never reaches: naming the writers in fetched
    /// thread history, and resolving an outbound `<@login>`. `None` for an
    /// adapter built without a directory, where both degrade to raw ids.
    users: Option<Arc<dyn ProviderIdentityLink>>,
    /// Workspace (team) id memoized after the first `auth.test` call so
    /// direct sends don't ping Slack on every send.
    workspace_id: OnceCell<String>,
}

impl SlackAdapter {
    /// Register this adapter's capabilities into `adapters`.
    ///
    /// Construction lives at the composition root; the provider owns the
    /// mapping from itself to the capability surfaces it implements.
    pub fn register_into(self: Arc<Self>, adapters: &mut Adapters) {
        let kind = AdapterKind::Slack;
        adapters.register_trait_object(self.clone());
        adapters.register_binding_reply(self.clone());
        adapters.register_message_reply(self.clone());
        adapters.register_direct_destination(self.clone());
        adapters.register_direct_message(self.clone());
        adapters.register_reactions(self.clone());
        adapters.register_inbound_presentation(kind, self);
    }

    pub fn new(bot_token: &str, signing_secret: &str, bot_user_id: Option<String>) -> Self {
        Self {
            api: SlackApi::new(bot_token),
            signing_secret: signing_secret.to_string(),
            bot_user_id,
            apps: None,
            users: None,
            workspace_id: OnceCell::new(),
        }
    }

    pub fn from_db(
        config: &Config,
        apps: Arc<dyn AdapterAppReader>,
        users: Arc<dyn ProviderIdentityLink>,
    ) -> Self {
        let bot_token = config.slack.bot_token.as_deref().unwrap_or("");
        Self {
            api: SlackApi::new(bot_token),
            signing_secret: config.slack.signing_secret.clone().unwrap_or_default(),
            bot_user_id: config.slack.bot_user_id.clone(),
            apps: Some(apps),
            users: Some(users),
            workspace_id: OnceCell::new(),
        }
    }

    /// Build an adapter around an already-constructed API client.
    ///
    /// [`Self::new`] builds its client internally, which leaves nowhere to put
    /// a stub; every seam test here goes through this constructor with a
    /// [`SlackApi::with_base_url`] client pointed at a loopback stub.
    #[cfg(test)]
    pub(super) fn with_api(
        api: SlackApi,
        signing_secret: &str,
        bot_user_id: Option<String>,
    ) -> Self {
        Self {
            api,
            signing_secret: signing_secret.to_string(),
            bot_user_id,
            apps: None,
            users: None,
            workspace_id: OnceCell::new(),
        }
    }

    /// The same, with a directory behind it, for the paths that name people.
    #[cfg(test)]
    pub(super) fn with_users(mut self, users: Arc<dyn ProviderIdentityLink>) -> Self {
        self.users = Some(users);
        self
    }

    pub(super) async fn app(&self, app_key: &str) -> Result<ResolvedSlackApp, AdapterError> {
        let Some(apps) = &self.apps else {
            return Ok(ResolvedSlackApp {
                api: self.api.clone(),
                signing_secret: self.signing_secret.clone(),
                bot_user_id: self.bot_user_id.clone(),
            });
        };
        let Some(app) = apps.app("slack", app_key).await? else {
            return Err(
                SharedAdapterError::AdapterNotConfigured(format!("slack app '{app_key}'")).into(),
            );
        };
        ResolvedSlackApp::try_from(app)
    }

    /// Return the default Slack workspace (team) ID, calling `auth.test` once
    /// and memoizing the result for the lifetime of the adapter.
    pub(super) async fn ensure_workspace_id(&self) -> Result<String, AdapterError> {
        self.workspace_id
            .get_or_try_init(|| async {
                let app = self.app(moatless_adapter_model::apps::DEFAULT_APP_KEY).await?;
                let info = app.api.auth_test().await?;
                info.team_id.ok_or_else(|| {
                    SharedAdapterError::AdapterApi("Slack auth.test returned no team_id".into())
                        .into()
                })
            })
            .await
            .cloned()
    }

    /// The bot's own Slack user ID, when it is known.
    pub(super) fn bot_user_id(&self) -> Option<&str> {
        self.bot_user_id.as_deref()
    }

    /// The Slack ↔ Moatless links, where this adapter was built with a pool.
    pub(super) fn users(&self) -> Option<&dyn ProviderIdentityLink> {
        self.users.as_deref()
    }
}

impl Adapter for SlackAdapter {
    fn adapter_kind(&self) -> AdapterKind {
        AdapterKind::Slack
    }
}

impl AdapterOutboundContent for SlackAdapter {
    fn supports_files(&self) -> bool {
        true
    }
}
