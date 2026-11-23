//! An admin setting the provider credential a bot user acts with.
//!
//! A bot cannot sign in to connect a provider itself, so its tasks hold no
//! credential until an admin sets one here. Each provider is one
//! [`BotCredentialProvider`] variant and one arm per operation below; the
//! route and the CLI take the provider as data.

use std::sync::Arc;

use moatless_core::http_auth::AuthUser;
use moatless_identity::forgejo::{ForgejoAccess, ForgejoProviderTokenStatus};
use moatless_identity::git_host_access::GitHostPrincipal;
use moatless_ports::identity::UserResolver;
use moatless_types::access_control::ScopeContext;
use tracing::info;

use crate::access_settings::error::AccessSettingsError;
use crate::access_settings::schemas::{
    BotCredentialProvider, BotProviderTokenStatusResponse, SaveBotProviderTokenRequest,
};

/// Reads and writes a bot user's provider credentials.
#[derive(Clone)]
pub struct BotCredentials {
    users: Arc<dyn UserResolver>,
    forgejo: Arc<ForgejoAccess>,
}

impl BotCredentials {
    pub(crate) fn new(users: Arc<dyn UserResolver>, forgejo: Arc<ForgejoAccess>) -> Self {
        Self { users, forgejo }
    }

    pub async fn status(
        &self,
        auth: &AuthUser,
        user: &str,
        provider: BotCredentialProvider,
        host: Option<&str>,
    ) -> Result<BotProviderTokenStatusResponse, AccessSettingsError> {
        let bot = bot_principal(auth, self.users.as_ref(), user).await?;
        match provider {
            BotCredentialProvider::Forgejo => {
                let host = required_host(provider, host)?;
                Ok(forgejo_status(
                    self.forgejo.status(&bot.user_id, host).await?,
                ))
            }
        }
    }

    pub async fn save(
        &self,
        auth: &AuthUser,
        user: &str,
        provider: BotCredentialProvider,
        request: SaveBotProviderTokenRequest,
    ) -> Result<BotProviderTokenStatusResponse, AccessSettingsError> {
        let bot = bot_principal(auth, self.users.as_ref(), user).await?;
        let status = match provider {
            BotCredentialProvider::Forgejo => {
                let host = required_host(provider, request.host.as_deref())?;
                forgejo_status(
                    self.forgejo
                        .save_personal_access_token(&bot, host, request.token)
                        .await?,
                )
            }
        };
        info!(
            admin_id = %auth.user_id,
            bot_user_id = %bot.user_id,
            provider = provider.as_str(),
            host = status.host.as_deref(),
            "Set a bot user's provider token"
        );
        Ok(status)
    }

    pub async fn remove(
        &self,
        auth: &AuthUser,
        user: &str,
        provider: BotCredentialProvider,
        host: Option<&str>,
    ) -> Result<BotProviderTokenStatusResponse, AccessSettingsError> {
        let bot = bot_principal(auth, self.users.as_ref(), user).await?;
        let status = match provider {
            BotCredentialProvider::Forgejo => {
                let host = required_host(provider, host)?;
                forgejo_status(self.forgejo.disconnect(&bot.user_id, host).await?)
            }
        };
        info!(
            admin_id = %auth.user_id,
            bot_user_id = %bot.user_id,
            provider = provider.as_str(),
            host = status.host.as_deref(),
            "Removed a bot user's provider token"
        );
        Ok(status)
    }
}

/// The bot `reference` names, as the principal its credential is written for.
///
/// Admin only, and only a bot: a person connects their own account in
/// Settings, so an admin's token set here would act under the person's name
/// rather than the account that owns the token.
async fn bot_principal(
    auth: &AuthUser,
    users: &dyn UserResolver,
    reference: &str,
) -> Result<GitHostPrincipal, AccessSettingsError> {
    if !auth.scope_context().is_admin {
        return Err(AccessSettingsError::AuthForbidden);
    }
    let user = users
        .run_as_target(reference)
        .await
        .map_err(AccessSettingsError::Store)?
        .ok_or_else(|| AccessSettingsError::NotFound(format!("No user '{reference}'")))?;
    if !user.is_bot {
        return Err(AccessSettingsError::BadRequest(format!(
            "'{}' is not a bot user; a person connects providers in their own Settings",
            user.login
        )));
    }
    let user_id = user.id.to_string();
    Ok(GitHostPrincipal {
        scope_context: ScopeContext::new(user_id.clone(), false),
        user_id,
    })
}

/// `host`, for a provider whose credentials are kept per instance.
fn required_host(
    provider: BotCredentialProvider,
    host: Option<&str>,
) -> Result<&str, AccessSettingsError> {
    host.filter(|host| !host.trim().is_empty()).ok_or_else(|| {
        AccessSettingsError::BadRequest(format!(
            "A {} credential is kept per instance; name its host",
            provider.as_str()
        ))
    })
}

fn forgejo_status(status: ForgejoProviderTokenStatus) -> BotProviderTokenStatusResponse {
    BotProviderTokenStatusResponse {
        provider: BotCredentialProvider::Forgejo,
        host: Some(status.host),
        connected: status.connected,
        auth_method: status.auth_method,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_ports::identity::{MockUserResolver, RunAsUser};

    fn caller(role: &str) -> AuthUser {
        AuthUser {
            user_id: uuid::Uuid::new_v4(),
            login: "caller".to_string(),
            provider: Some("local".to_string()),
            role: Some(role.to_string()),
            group_id: None,
            task_id: None,
            is_agent_context: false,
        }
    }

    fn resolving(user: Option<RunAsUser>) -> MockUserResolver {
        let mut users = MockUserResolver::new();
        users
            .expect_run_as_target()
            .returning(move |_| Ok(user.clone()));
        users
    }

    fn user(is_bot: bool) -> RunAsUser {
        RunAsUser {
            id: uuid::Uuid::new_v4(),
            login: if is_bot {
                "soap-agentops[bot]"
            } else {
                "alice"
            }
            .to_string(),
            is_bot,
        }
    }

    #[tokio::test]
    async fn a_bot_credential_is_written_as_the_bot_not_the_admin() {
        let bot = user(true);
        let principal = bot_principal(&caller("admin"), &resolving(Some(bot.clone())), "bot")
            .await
            .expect("an admin may set a bot's credential");

        assert_eq!(principal.user_id, bot.id.to_string());
        assert_eq!(principal.scope_context.user_id, bot.id.to_string());
        assert!(!principal.scope_context.is_admin);
    }

    #[tokio::test]
    async fn only_an_admin_sets_a_bot_credential() {
        let mut users = MockUserResolver::new();
        users.expect_run_as_target().never();
        assert!(matches!(
            bot_principal(&caller("user"), &users, "bot").await,
            Err(AccessSettingsError::AuthForbidden)
        ));
    }

    #[tokio::test]
    async fn an_admin_cannot_set_a_person_credential() {
        assert!(matches!(
            bot_principal(&caller("admin"), &resolving(Some(user(false))), "alice").await,
            Err(AccessSettingsError::BadRequest(_))
        ));
        assert!(matches!(
            bot_principal(&caller("admin"), &resolving(None), "nobody").await,
            Err(AccessSettingsError::NotFound(_))
        ));
    }

    #[test]
    fn a_forgejo_credential_needs_a_host() {
        let forgejo = BotCredentialProvider::Forgejo;
        assert!(matches!(
            required_host(forgejo, None),
            Err(AccessSettingsError::BadRequest(_))
        ));
        assert!(matches!(
            required_host(forgejo, Some(" ")),
            Err(AccessSettingsError::BadRequest(_))
        ));
        assert_eq!(
            required_host(forgejo, Some("git.acme.dev")).ok(),
            Some("git.acme.dev")
        );
    }
}
