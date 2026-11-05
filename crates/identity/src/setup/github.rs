//! The GitHub-token requirement.

use async_trait::async_trait;
use tracing::warn;

use super::evaluator::{SetupContext, SetupRequirementCheck};
use super::schemas::{
    SetupAction, SetupActionKind, SetupRequirement, SetupRequirementId, SetupRequirementStatus,
    SetupSeverity,
};
use crate::auth::credentials::GITHUB_OAUTH_STASH_KEY;
use crate::auth::models::GithubTokenPresence;
use moatless_core::secret_keys::GITHUB_TOKEN;
use moatless_types::access_control::Scope;
use moatless_types::secret::SecretKind;

/// Settings-dialog section holding the per-user GitHub connection.
const SETTINGS_SECTION: &str = moatless_types::git_host::RepositoryProvider::Github.as_str();

/// Without a GitHub token the app can't read repositories, so nothing else
/// works — repo listing and task creation both fail.
pub struct GithubTokenRequirement;

#[async_trait]
impl SetupRequirementCheck for GithubTokenRequirement {
    async fn evaluate(&self, ctx: &SetupContext<'_>) -> SetupRequirement {
        let status = match ctx
            .provider_tokens
            .find_github_scope_presence(ctx.user_id)
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
                actions()
            },
        }
    }
}

/// Turn presence into a status without decrypting anything or calling GitHub.
///
/// `/me` runs on every page load, so this stays to at most one extra indexed
/// read (the `secret_exists` probe), and only for reference rows.
async fn classify(
    ctx: &SetupContext<'_>,
    presence: &GithubTokenPresence,
) -> SetupRequirementStatus {
    let user_id = ctx.user_id;
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
    // A refused PAT is never presented, only the App sign-in stashed behind
    // it; with none, the remedy is connecting through the App.
    let (key, kind, absent) = if ctx.github_pats_disabled {
        (
            GITHUB_OAUTH_STASH_KEY,
            SecretKind::Internal,
            SetupRequirementStatus::Missing,
        )
    } else {
        (
            GITHUB_TOKEN,
            SecretKind::ProviderToken,
            SetupRequirementStatus::Broken,
        )
    };
    match secrets
        .secret_exists(Scope::User, Some(user_id), key, Some(kind))
        .await
    {
        Ok(true) => SetupRequirementStatus::Satisfied,
        Ok(false) => absent,
        Err(e) => {
            warn!(
                error = %e,
                user_id,
                "Failed to probe GitHub token secret while evaluating setup"
            );
            SetupRequirementStatus::Broken
        }
    }
}

/// One remedy, and it is the same for everyone: connect your own account. An
/// administrator's missing credential is their own to connect.
fn actions() -> Vec<SetupAction> {
    vec![SetupAction {
        kind: SetupActionKind::OpenSettings,
        target: SETTINGS_SECTION.to_string(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::provider_tokens::SECRET_REF_TOKEN_PLACEHOLDER;
    use crate::auth::test_support::{StubProviderTokens, github_row};
    use crate::secret::service::SecretService;
    use crate::secret::test_support::{secret, service_with};

    async fn evaluate(
        tokens: &StubProviderTokens,
        secrets: Option<&SecretService>,
        is_admin: bool,
    ) -> SetupRequirement {
        evaluate_with(tokens, secrets, is_admin, false).await
    }

    async fn evaluate_with(
        tokens: &StubProviderTokens,
        secrets: Option<&SecretService>,
        is_admin: bool,
        github_pats_disabled: bool,
    ) -> SetupRequirement {
        GithubTokenRequirement
            .evaluate(&SetupContext {
                user_id: "u1",
                is_admin,
                provider_tokens: tokens,
                secrets,
                github_pats_disabled,
            })
            .await
    }

    #[tokio::test]
    async fn no_row_at_any_scope_is_missing() {
        let req = evaluate(&StubProviderTokens::default(), None, false).await;
        assert_eq!(req.status, SetupRequirementStatus::Missing);
    }

    #[tokio::test]
    async fn a_plain_user_row_satisfies_and_offers_no_actions() {
        let tokens = StubProviderTokens::with_user_row(Some(github_row("oauth", "gho_x")));
        let req = evaluate(&tokens, None, false).await;
        assert_eq!(req.status, SetupRequirementStatus::Satisfied);
        assert!(req.actions.is_empty());
    }

    #[tokio::test]
    async fn an_org_wide_row_does_not_satisfy_a_user_with_none() {
        // Their Sandbox resolves only their own credential, so reporting them
        // as set up sends them to a Task that fails at preflight.
        let tokens = StubProviderTokens::default().with_global_row(github_row("manual", "ghp_org"));
        let req = evaluate(&tokens, None, false).await;
        assert_eq!(req.status, SetupRequirementStatus::Missing);
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
        let req = evaluate(&tokens, Some(&secrets), false).await;
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
        let req = evaluate(&tokens, Some(&secrets), false).await;
        assert_eq!(req.status, SetupRequirementStatus::Broken);
    }

    #[tokio::test]
    async fn a_refused_pat_is_set_up_only_by_the_app_sign_in_behind_it() {
        let tokens = StubProviderTokens::with_user_row(Some(github_row(
            crate::auth::provider_tokens::AUTH_METHOD_SECRET_REF,
            SECRET_REF_TOKEN_PLACEHOLDER,
        )));
        let pat = secret(
            "user",
            Some("u1"),
            "provider_token",
            GITHUB_TOKEN,
            "ghp_real",
        );
        let stash = secret(
            "user",
            Some("u1"),
            "internal",
            GITHUB_OAUTH_STASH_KEY,
            "ghu_stashed",
        );

        let pat_only = service_with(vec![pat.clone()]);
        let req = evaluate_with(&tokens, Some(&pat_only), false, true).await;
        assert_eq!(
            req.status,
            SetupRequirementStatus::Missing,
            "the PAT alone is refused, so connecting through the App is the remedy"
        );
        assert_eq!(req.actions.len(), 1);

        let behind = service_with(vec![pat, stash]);
        let req = evaluate_with(&tokens, Some(&behind), false, true).await;
        assert_eq!(req.status, SetupRequirementStatus::Satisfied);
    }

    #[tokio::test]
    async fn an_admin_is_offered_the_same_remedy_as_anybody_else() {
        for is_admin in [true, false] {
            let req = evaluate(&StubProviderTokens::default(), None, is_admin).await;
            assert_eq!(req.actions.len(), 1, "is_admin={is_admin}");
            assert_eq!(req.actions[0].kind, SetupActionKind::OpenSettings);
        }
    }
}
