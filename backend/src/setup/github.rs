//! The GitHub-token requirement.

use async_trait::async_trait;
use tracing::warn;

use super::evaluator::{SetupContext, SetupRequirementCheck};
use super::schemas::{
    SetupAction, SetupActionKind, SetupRequirement, SetupRequirementId, SetupRequirementStatus,
    SetupSeverity,
};
use crate::auth::credentials::secret_scope_for;
use crate::auth::models::GithubTokenPresence;
use crate::credentials::github::scope::GitHubScope;
use crate::sandbox::secrets::GITHUB_TOKEN;
use crate::secret::models::SecretKind;

/// Settings-dialog section holding the per-user GitHub connection.
const SETTINGS_SECTION: &str = "github";
/// Admin page for the org-wide GitHub token.
const ADMIN_ROUTE: &str = "/admin/integrations/github";

/// Without a GitHub token the app can't read repositories, so nothing else
/// works — repo listing and task creation both fail.
pub struct GithubTokenRequirement;

#[async_trait]
impl SetupRequirementCheck for GithubTokenRequirement {
    async fn evaluate(&self, ctx: &SetupContext<'_>) -> SetupRequirement {
        let status = match ctx
            .provider_tokens
            .find_github_scope_presence(ctx.user_id, ctx.team_ids)
            .await
        {
            Ok(Some(presence)) => classify(ctx, &presence).await,
            Ok(None) => SetupRequirementStatus::Missing,
            Err(e) => {
                warn!(
                    error = %e,
                    user_id = %ctx.user_id,
                    "Failed to read GitHub token presence while evaluating setup"
                );
                SetupRequirementStatus::Broken
            }
        };

        SetupRequirement {
            id: SetupRequirementId::GithubToken,
            status,
            severity: SetupSeverity::Required,
            actions: if status == SetupRequirementStatus::Satisfied {
                Vec::new()
            } else {
                actions(ctx.is_admin)
            },
        }
    }
}

/// Turn presence into a status without decrypting anything or calling GitHub.
///
/// `/me` runs on every page load, so this stays to at most one extra indexed
/// read — the `secret_exists` probe, and only for reference rows.
async fn classify(ctx: &SetupContext<'_>, presence: &GithubTokenPresence) -> SetupRequirementStatus {
    if presence.refresh_failure {
        return SetupRequirementStatus::Broken;
    }

    if presence.auth_method.as_deref() != Some(crate::auth::provider_tokens::AUTH_METHOD_SECRET_REF)
    {
        return SetupRequirementStatus::Satisfied;
    }

    // A reference row promises an encrypted PAT. If the secret is gone the
    // user *is* connected but the credential is unreachable — `Broken`, not
    // `Missing`, so the frontend doesn't tell them to connect again.
    let Some(secrets) = ctx.secrets else {
        return SetupRequirementStatus::Broken;
    };
    let scope = GitHubScope::from_row(
        &presence.scope,
        presence.user_id.as_deref(),
        presence.team_id.as_deref(),
    );
    let (scope_type, scope_id) = secret_scope_for(&scope);

    match secrets
        .secret_exists(
            scope_type,
            scope_id,
            GITHUB_TOKEN,
            Some(SecretKind::ProviderToken),
        )
        .await
    {
        Ok(true) => SetupRequirementStatus::Satisfied,
        Ok(false) => SetupRequirementStatus::Broken,
        Err(e) => {
            warn!(
                error = %e,
                scope = %scope,
                "Failed to probe GitHub token secret while evaluating setup"
            );
            SetupRequirementStatus::Broken
        }
    }
}

/// Admins get the org-wide route first: setting one token there fixes it for
/// everyone, which beats each user pasting their own PAT.
fn actions(is_admin: bool) -> Vec<SetupAction> {
    let mut actions = Vec::with_capacity(2);
    if is_admin {
        actions.push(SetupAction {
            kind: SetupActionKind::AdminRoute,
            target: ADMIN_ROUTE.to_string(),
        });
    }
    actions.push(SetupAction {
        kind: SetupActionKind::OpenSettings,
        target: SETTINGS_SECTION.to_string(),
    });
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::provider_tokens::SECRET_REF_TOKEN_PLACEHOLDER;
    use crate::auth::test_support::{github_row, StubProviderTokens};
    use crate::secret::service::SecretService;
    use crate::secret::test_support::{secret, service_with};

    async fn evaluate(
        tokens: &StubProviderTokens,
        secrets: Option<&SecretService>,
        is_admin: bool,
        team_ids: &[String],
    ) -> SetupRequirement {
        GithubTokenRequirement
            .evaluate(&SetupContext {
                user_id: "u1",
                team_ids,
                is_admin,
                provider_tokens: tokens,
                secrets,
            })
            .await
    }

    #[tokio::test]
    async fn no_row_at_any_scope_is_missing() {
        let req = evaluate(&StubProviderTokens::default(), None, false, &[]).await;
        assert_eq!(req.status, SetupRequirementStatus::Missing);
    }

    #[tokio::test]
    async fn a_plain_user_row_satisfies_and_offers_no_actions() {
        let tokens = StubProviderTokens::with_user_row(Some(github_row("oauth", "gho_x")));
        let req = evaluate(&tokens, None, false, &[]).await;
        assert_eq!(req.status, SetupRequirementStatus::Satisfied);
        assert!(req.actions.is_empty());
    }

    #[tokio::test]
    async fn the_org_wide_token_satisfies_a_user_with_none() {
        let tokens = StubProviderTokens::default().with_global_row(github_row("manual", "ghp_org"));
        let req = evaluate(&tokens, None, false, &[]).await;
        assert_eq!(req.status, SetupRequirementStatus::Satisfied);
    }

    #[tokio::test]
    async fn a_secret_ref_row_with_its_secret_intact_is_satisfied() {
        let tokens = StubProviderTokens::with_user_row(Some(github_row(
            crate::auth::provider_tokens::AUTH_METHOD_SECRET_REF,
            SECRET_REF_TOKEN_PLACEHOLDER,
        )));
        let secrets = service_with(vec![secret(
            "user",
            Some("u1"),
            "provider_token",
            GITHUB_TOKEN,
            "ghp_real",
        )]);
        let req = evaluate(&tokens, Some(&secrets), false, &[]).await;
        assert_eq!(req.status, SetupRequirementStatus::Satisfied);
    }

    #[tokio::test]
    async fn a_dangling_secret_ref_is_broken_not_missing() {
        // "Connect GitHub" would be the wrong prompt here — the user did
        // connect, the stored credential just isn't reachable.
        let tokens = StubProviderTokens::with_user_row(Some(github_row(
            crate::auth::provider_tokens::AUTH_METHOD_SECRET_REF,
            SECRET_REF_TOKEN_PLACEHOLDER,
        )));
        let secrets = service_with(vec![]);
        let req = evaluate(&tokens, Some(&secrets), false, &[]).await;
        assert_eq!(req.status, SetupRequirementStatus::Broken);
    }

    #[tokio::test]
    async fn admins_are_offered_the_org_wide_route_first() {
        let req = evaluate(&StubProviderTokens::default(), None, true, &[]).await;
        assert_eq!(req.actions[0].kind, SetupActionKind::AdminRoute);
        assert_eq!(req.actions[0].target, ADMIN_ROUTE);
        assert_eq!(req.actions[1].kind, SetupActionKind::OpenSettings);
    }

    #[tokio::test]
    async fn non_admins_are_never_offered_the_admin_route() {
        let req = evaluate(&StubProviderTokens::default(), None, false, &[]).await;
        assert_eq!(req.actions.len(), 1);
        assert_eq!(req.actions[0].kind, SetupActionKind::OpenSettings);
    }
}
