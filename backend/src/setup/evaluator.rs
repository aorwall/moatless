//! Requirement registry and evaluation.
//!
//! Adding a requirement is one new module implementing
//! [`SetupRequirementCheck`] plus one line in [`checks`].

use async_trait::async_trait;

use super::github::GithubTokenRequirement;
use super::schemas::{SetupRequirement, SetupRequirementStatus, SetupSeverity, SetupStatus};
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::secret::service::SecretService;

/// Everything a requirement check may read.
///
/// An explicit dependency list rather than a state handle, because `/me` runs
/// on `AuthState` while most of the app runs on `AppState` — the evaluator has
/// to work from either.
pub struct SetupContext<'a> {
    pub user_id: &'a str,
    pub team_ids: &'a [String],
    pub is_admin: bool,
    pub provider_tokens: &'a dyn ProviderTokenRepository,
    pub secrets: Option<&'a SecretService>,
}

#[async_trait]
pub trait SetupRequirementCheck: Send + Sync {
    /// Must never fail: `/me` is the session endpoint, and a DB hiccup while
    /// evaluating an advisory banner must not log the user out. Infrastructure
    /// errors are logged and reported as [`SetupRequirementStatus::Broken`].
    async fn evaluate(&self, ctx: &SetupContext<'_>) -> SetupRequirement;
}

fn checks() -> Vec<Box<dyn SetupRequirementCheck>> {
    vec![Box::new(GithubTokenRequirement)]
}

/// Evaluate every requirement for this user.
pub async fn evaluate_setup(ctx: &SetupContext<'_>) -> SetupStatus {
    let mut requirements = Vec::new();
    for check in checks() {
        requirements.push(check.evaluate(ctx).await);
    }

    let complete = requirements.iter().all(|r| {
        r.severity != SetupSeverity::Required || r.status == SetupRequirementStatus::Satisfied
    });

    SetupStatus {
        complete,
        requirements,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::{github_row, StubProviderTokens};

    fn context<'a>(tokens: &'a StubProviderTokens, is_admin: bool) -> SetupContext<'a> {
        SetupContext {
            user_id: "u1",
            team_ids: &[],
            is_admin,
            provider_tokens: tokens,
            secrets: None,
        }
    }

    #[tokio::test]
    async fn a_user_with_no_github_token_is_not_set_up() {
        let tokens = StubProviderTokens::default();
        let status = evaluate_setup(&context(&tokens, false)).await;
        assert!(!status.complete);
        assert_eq!(status.requirements.len(), 1);
        assert_eq!(
            status.requirements[0].status,
            SetupRequirementStatus::Missing
        );
    }

    #[tokio::test]
    async fn satisfied_requirements_are_still_listed() {
        // The frontend renders a checklist, not just a nag — a satisfied
        // requirement has to stay in the array.
        let tokens = StubProviderTokens::with_user_row(Some(github_row("oauth", "gho_x")));
        let status = evaluate_setup(&context(&tokens, false)).await;
        assert!(status.complete);
        assert_eq!(status.requirements.len(), 1);
        assert_eq!(
            status.requirements[0].status,
            SetupRequirementStatus::Satisfied
        );
    }
}
