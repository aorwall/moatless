//! Reaction capability for GitHub pull request comments.

use moatless_adapter_model::binding::BindingContext;
use moatless_adapter_model::capabilities::AdapterReactions;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_workspace_types::repository::GitHostRepo;

use super::adapter::GithubPrAdapter;
use super::external_id::{self, CommentTarget};
use super::subject::GithubRepository;

/// Map a user-supplied emoji name to GitHub's reaction content enum.
///
/// Accepts GitHub's canonical values plus a few common aliases.
fn to_github_content(emoji: &str) -> Result<&'static str, AdapterError> {
    let normalized = emoji.trim().trim_matches(':').to_ascii_lowercase();
    let mapped = match normalized.as_str() {
        "+1" | "thumbsup" | "thumbs_up" => "+1",
        "-1" | "thumbsdown" | "thumbs_down" => "-1",
        "laugh" | "laughing" | "smile" => "laugh",
        "confused" | "thinking" | "thinking_face" => "confused",
        "heart" | "love" | "red_heart" => "heart",
        "hooray" | "tada" | "party" => "hooray",
        "rocket" => "rocket",
        "eyes" | "looking" => "eyes",
        other => {
            return Err(SharedAdapterError::BadRequest(format!(
                "GitHub does not accept emoji `{other}` (allowed: +1, -1, laugh, confused, heart, hooray, rocket, eyes)"
            )).into());
        }
    };
    Ok(mapped)
}

#[async_trait::async_trait]
impl AdapterReactions for GithubPrAdapter {
    async fn add_reaction(
        &self,
        binding: &BindingContext,
        acting_user_id: &str,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<Option<String>, AdapterError> {
        let target = external_id::comment_target_from_id(target_external_id).ok_or_else(|| {
            SharedAdapterError::BadRequest(format!(
                "GitHub reaction target `{target_external_id}` must be a github:ic:<id> or github:rc:<id> id"
            ))
        })?;
        let content = to_github_content(emoji)?;
        let repository = GithubRepository::from_binding_context(binding)?;
        let api = self
            .auth()
            .client_for_reaction(
                binding,
                acting_user_id,
                &GitHostRepo::github(repository.owner(), repository.repo()),
            )
            .await?;
        let reaction_id = match target {
            CommentTarget::Issue(id) => {
                api.create_issue_comment_reaction(
                    repository.owner(),
                    repository.repo(),
                    id,
                    content,
                )
                .await?
            }
            CommentTarget::Review(id) => {
                api.create_review_comment_reaction(
                    repository.owner(),
                    repository.repo(),
                    id,
                    content,
                )
                .await?
            }
        };
        Ok(Some(reaction_id.to_string()))
    }

    async fn remove_reaction(
        &self,
        binding: &BindingContext,
        acting_user_id: &str,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<(), AdapterError> {
        let target = external_id::comment_target_from_id(target_external_id).ok_or_else(|| {
            SharedAdapterError::BadRequest(format!(
                "GitHub reaction target `{target_external_id}` must be a github:ic:<id> or github:rc:<id> id"
            ))
        })?;
        let content = to_github_content(emoji)?;
        let repository = GithubRepository::from_binding_context(binding)?;
        let api = self
            .auth()
            .client_for_reaction(
                binding,
                acting_user_id,
                &GitHostRepo::github(repository.owner(), repository.repo()),
            )
            .await?;
        let bot_login = api.authenticated_login().await?;
        let (reactions, comment_id, is_review) = match target {
            CommentTarget::Issue(id) => (
                api.list_issue_comment_reactions(repository.owner(), repository.repo(), id)
                    .await?,
                id,
                false,
            ),
            CommentTarget::Review(id) => (
                api.list_review_comment_reactions(repository.owner(), repository.repo(), id)
                    .await?,
                id,
                true,
            ),
        };
        let reaction = reactions
            .into_iter()
            .find(|reaction| {
                reaction.content == content
                    && reaction
                        .user
                        .as_ref()
                        .is_some_and(|user| user.login == bot_login)
            })
            .ok_or_else(|| {
                SharedAdapterError::NotFound(format!(
                    "GitHub: no `{content}` reaction by `{bot_login}` to remove on comment {comment_id}"
                ))
            })?;
        if is_review {
            api.delete_review_comment_reaction(
                repository.owner(),
                repository.repo(),
                comment_id,
                reaction.id,
            )
            .await?;
        } else {
            api.delete_issue_comment_reaction(
                repository.owner(),
                repository.repo(),
                comment_id,
                reaction.id,
            )
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::{Method, StatusCode};

    use super::super::test_support::{StubResponse, start_github_api_stub};
    use super::*;
    use moatless_adapter_model::binding::BindingContext;
    use moatless_identity::auth::test_support::{StubProviderTokens, github_row};

    #[test]
    fn github_content_accepts_canonical_and_aliases() {
        assert_eq!(to_github_content("+1").unwrap(), "+1");
        assert_eq!(to_github_content("thumbsup").unwrap(), "+1");
        assert_eq!(to_github_content(":thumbsup:").unwrap(), "+1");
        assert_eq!(to_github_content("THUMBSDOWN").unwrap(), "-1");
        assert_eq!(to_github_content("rocket").unwrap(), "rocket");
        assert_eq!(to_github_content("tada").unwrap(), "hooray");
        assert_eq!(to_github_content("eyes").unwrap(), "eyes");
    }

    #[test]
    fn github_content_rejects_unknown_names() {
        let err = to_github_content("celebration").unwrap_err();
        assert!(matches!(
            err,
            AdapterError::Shared(SharedAdapterError::BadRequest(_))
        ));
    }

    #[tokio::test]
    async fn reaction_capabilities_keep_exact_github_request_budgets() {
        let stub = start_github_api_stub(|method, path, uri| match (method, path) {
            (Method::POST, "/repos/soaplabs/repo/issues/comments/77/reactions") => {
                StubResponse::json(StatusCode::CREATED, serde_json::json!({"id": 103}))
            }
            (Method::GET, "/user") => {
                StubResponse::json(StatusCode::OK, serde_json::json!({"login": "moatless-bot"}))
            }
            (Method::GET, "/repos/soaplabs/repo/pulls/comments/88/reactions") => {
                StubResponse::json(
                    StatusCode::OK,
                    serde_json::json!([{
                        "id": 104,
                        "content": "+1",
                        "user": {"login": "moatless-bot"}
                    }]),
                )
            }
            (Method::DELETE, "/repos/soaplabs/repo/pulls/comments/88/reactions/104") => {
                StubResponse::empty(StatusCode::NO_CONTENT)
            }
            _ => StubResponse::not_found(uri),
        })
        .await;

        let provider = provider(stub.base_url());
        let binding = binding_context(Some("owner-user"));

        assert_eq!(
            provider
                .add_reaction(&binding, "caller", "github:ic:77", "thumbsup")
                .await
                .unwrap()
                .as_deref(),
            Some("103")
        );
        provider
            .remove_reaction(&binding, "caller", "github:rc:88", "thumbsup")
            .await
            .unwrap();

        // Every request below was provoked by this test: the stub is reached
        // only by the client this Adapter built against it.
        let recorded = stub.recorded_requests().await;
        let selected_reaction_token = Some("Bearer owner-token");
        let reaction_requests: Vec<_> = recorded.iter().collect();
        let observed: Vec<(&str, &str)> = reaction_requests
            .iter()
            .map(|request| (request.method.as_str(), request.uri.as_str()))
            .collect();
        assert_eq!(
            observed,
            vec![
                ("POST", "/repos/soaplabs/repo/issues/comments/77/reactions"),
                ("GET", "/user"),
                (
                    "GET",
                    "/repos/soaplabs/repo/pulls/comments/88/reactions?per_page=100"
                ),
                (
                    "DELETE",
                    "/repos/soaplabs/repo/pulls/comments/88/reactions/104"
                ),
            ]
        );
        assert!(
            reaction_requests
                .iter()
                .all(|request| request.authorization.as_deref() == selected_reaction_token),
            "reaction calls keep their selected user token: {recorded:?}"
        );
    }

    fn provider(api_base: &str) -> GithubPrAdapter {
        GithubPrAdapter::new(
            None,
            "https://app.example.com".to_string(),
            moatless_core::config::DEFAULT_GITHUB_API_BASE.to_string(),
            moatless_identity::git_host_tokens::seam_over(Arc::new(
                StubProviderTokens::with_user_row_for(
                    "owner-user",
                    github_row("manual", "owner-token"),
                ),
            )),
        )
        .with_api_base(api_base)
    }

    fn binding_context(filter_user_id: Option<&str>) -> BindingContext {
        let now = chrono::Utc::now();
        BindingContext {
            binding_id: "tb_test".to_string(),
            loop_id: None,
            task_id: "task_test".to_string(),
            subject_kind: "github_pr".to_string(),
            subject_key: "soaplabs/repo#42".to_string(),
            destination_config: serde_json::json!({
                "owner": "soaplabs",
                "repo": "repo",
                "pr_number": 42,
                "filter_user_id": filter_user_id,
            }),
            metadata: serde_json::json!({"filter_user_id": filter_user_id}),
            last_external_event_id: None,
            last_message_at: now,
            created_at: now,
        }
    }
}
