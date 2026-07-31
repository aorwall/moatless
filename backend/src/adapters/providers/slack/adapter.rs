//! The Slack Adapter itself: its construction state, and the two capabilities
//! that are properties of the Adapter rather than behavior of a workflow.

use tokio::sync::OnceCell;

use crate::adapters::error::AdapterError;
use crate::adapters::{Adapter, AdapterKind, AdapterOutboundContent};

use super::api::SlackApi;

/// Slack adapter.
pub struct SlackAdapter {
    api: SlackApi,
    signing_secret: String,
    bot_user_id: Option<String>,
    /// Workspace (team) id memoized after the first `auth.test` call so
    /// `post_to_destination` doesn't ping Slack on every send.
    workspace_id: OnceCell<String>,
}

impl SlackAdapter {
    pub fn new(bot_token: &str, signing_secret: &str, bot_user_id: Option<String>) -> Self {
        Self {
            api: SlackApi::new(bot_token),
            signing_secret: signing_secret.to_string(),
            bot_user_id,
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
            workspace_id: OnceCell::new(),
        }
    }

    /// Return the Slack workspace (team) ID, calling `auth.test` once and
    /// memoizing the result for the lifetime of the provider.
    pub(super) async fn ensure_workspace_id(&self) -> Result<String, AdapterError> {
        self.workspace_id
            .get_or_try_init(|| async {
                let info = self.api.auth_test().await?;
                info.team_id.ok_or_else(|| {
                    AdapterError::AdapterApi("Slack auth.test returned no team_id".into())
                })
            })
            .await
            .cloned()
    }

    /// The secret every inbound request is verified against.
    pub(super) fn signing_secret(&self) -> &str {
        &self.signing_secret
    }

    /// The Slack Web API client, for the workflow modules that split off this
    /// file.
    pub(super) fn api(&self) -> &SlackApi {
        &self.api
    }

    /// The bot's own Slack user ID, when it is known.
    pub(super) fn bot_user_id(&self) -> Option<&str> {
        self.bot_user_id.as_deref()
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
