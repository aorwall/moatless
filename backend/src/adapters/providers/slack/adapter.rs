//! The Slack Adapter itself: its construction state, and the two capabilities
//! that are properties of the Adapter rather than behavior of a workflow.

use std::sync::Arc;

use tokio::sync::OnceCell;

use crate::adapters::error::AdapterError;
use crate::adapters::{Adapter, AdapterKind, AdapterOutboundContent};
use crate::config::Config;
use crate::db::DbPool;

use super::api::SlackApi;

pub(super) struct ResolvedSlackApp {
    pub(super) api: SlackApi,
    pub(super) signing_secret: String,
    pub(super) bot_user_id: Option<String>,
}

/// Slack adapter.
pub struct SlackAdapter {
    api: SlackApi,
    signing_secret: String,
    bot_user_id: Option<String>,
    db: Option<DbPool>,
    config: Option<Arc<Config>>,
    /// Workspace (team) id memoized after the first `auth.test` call so
    /// direct sends don't ping Slack on every send.
    workspace_id: OnceCell<String>,
}

impl SlackAdapter {
    pub fn new(bot_token: &str, signing_secret: &str, bot_user_id: Option<String>) -> Self {
        Self {
            api: SlackApi::new(bot_token),
            signing_secret: signing_secret.to_string(),
            bot_user_id,
            db: None,
            config: None,
            workspace_id: OnceCell::new(),
        }
    }

    pub fn from_db(config: Arc<Config>, db: DbPool) -> Self {
        let bot_token = config.slack.bot_token.as_deref().unwrap_or("");
        Self {
            api: SlackApi::new(bot_token),
            signing_secret: config.slack.signing_secret.clone().unwrap_or_default(),
            bot_user_id: config.slack.bot_user_id.clone(),
            db: Some(db),
            config: Some(config),
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
            db: None,
            config: None,
            workspace_id: OnceCell::new(),
        }
    }

    pub(super) async fn app(&self, app_key: &str) -> Result<ResolvedSlackApp, AdapterError> {
        let Some(db) = &self.db else {
            return Ok(ResolvedSlackApp {
                api: self.api.clone(),
                signing_secret: self.signing_secret.clone(),
                bot_user_id: self.bot_user_id.clone(),
            });
        };
        let config = self
            .config
            .as_deref()
            .ok_or_else(|| AdapterError::Internal("Slack Adapter missing config".into()))?;
        let Some(app) = crate::adapter_apps::slack_app_config(db, config, app_key)
            .await
            .map_err(|e| AdapterError::AdapterApi(e.to_string()))?
        else {
            return Err(AdapterError::AdapterNotConfigured(format!(
                "slack app '{app_key}'"
            )));
        };
        Ok(ResolvedSlackApp {
            api: SlackApi::new(&app.bot_token),
            signing_secret: app.signing_secret,
            bot_user_id: app.bot_user_id,
        })
    }

    /// Return the default Slack workspace (team) ID, calling `auth.test` once
    /// and memoizing the result for the lifetime of the adapter.
    pub(super) async fn ensure_workspace_id(&self) -> Result<String, AdapterError> {
        self.workspace_id
            .get_or_try_init(|| async {
                let app = self.app(crate::adapter_apps::DEFAULT_APP_KEY).await?;
                let info = app.api.auth_test().await?;
                info.team_id.ok_or_else(|| {
                    AdapterError::AdapterApi("Slack auth.test returned no team_id".into())
                })
            })
            .await
            .cloned()
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
