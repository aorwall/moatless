//! Reaction capability for GitHub pull request comments.

use crate::adapters::AdapterReactions;
use crate::adapters::error::AdapterError;
use crate::adapters::models::BindingContext;

use super::adapter::GithubPrAdapter;
use super::external_id::{self, CommentTarget};
use super::subject::GithubRepository;

/// Map a user-supplied emoji name to GitHub's reaction content enum.
///
/// GitHub accepts only these eight values:
/// `+1`, `-1`, `laugh`, `confused`, `heart`, `hooray`, `rocket`, `eyes`.
/// We accept the canonical names plus a few common aliases.
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
            return Err(AdapterError::BadRequest(format!(
                "GitHub does not accept emoji `{other}` (allowed: +1, -1, laugh, confused, heart, hooray, rocket, eyes)"
            )));
        }
    };
    Ok(mapped)
}

#[async_trait::async_trait]
impl AdapterReactions for GithubPrAdapter {
    async fn add_reaction(
        &self,
        binding: &BindingContext,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<Option<String>, AdapterError> {
        let target = external_id::comment_target_from_id(target_external_id).ok_or_else(|| {
            AdapterError::BadRequest(format!(
                "GitHub reaction target `{target_external_id}` must be a github:ic:<id> or github:rc:<id> id"
            ))
        })?;
        let content = to_github_content(emoji)?;
        let repository = GithubRepository::from_binding_context(binding)?;
        let api = self
            .auth()
            .client_for_reaction(binding, repository.owner())
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
        target_external_id: &str,
        emoji: &str,
    ) -> Result<(), AdapterError> {
        let target = external_id::comment_target_from_id(target_external_id).ok_or_else(|| {
            AdapterError::BadRequest(format!(
                "GitHub reaction target `{target_external_id}` must be a github:ic:<id> or github:rc:<id> id"
            ))
        })?;
        let content = to_github_content(emoji)?;
        let repository = GithubRepository::from_binding_context(binding)?;
        let api = self
            .auth()
            .client_for_reaction(binding, repository.owner())
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
                AdapterError::NotFound(format!(
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
    use crate::adapters::models::BindingContext;
    use crate::auth::github::registry::GitHubAppRegistry;
    use crate::auth::test_support::{StubProviderTokens, github_row};

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
        assert!(matches!(err, AdapterError::BadRequest(_)));
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

        let provider = provider();
        let binding = binding_context(Some("owner-user"));

        assert_eq!(
            provider
                .add_reaction(&binding, "github:ic:77", "thumbsup")
                .await
                .unwrap()
                .as_deref(),
            Some("103")
        );
        provider
            .remove_reaction(&binding, "github:rc:88", "thumbsup")
            .await
            .unwrap();

        let recorded = stub.recorded_requests().await;
        let selected_reaction_token = Some("Bearer owner-token");
        // A `/app` probe with the selected reaction token belongs to this
        // reaction path, not to unrelated concurrent credential discovery.
        let selected_token_app_requests: Vec<_> = recorded
            .iter()
            .filter(|request| {
                request.uri == "/app" && request.authorization.as_deref() == selected_reaction_token
            })
            .collect();
        assert!(
            selected_token_app_requests.is_empty(),
            "reaction calls must not probe the GitHub App endpoint with the selected reaction token: {recorded:?}"
        );
        // The env-based GitHub API stub can receive unrelated App probes from
        // concurrent credential tests; keep this assertion scoped to reactions.
        let reaction_requests: Vec<_> = recorded
            .iter()
            .filter(|request| request.uri != "/app")
            .collect();
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

    fn provider() -> GithubPrAdapter {
        GithubPrAdapter::new(
            None,
            Some("wrong-global-token".to_string()),
            "https://app.example.com".to_string(),
            fake_db_pool(),
            Arc::new(StubProviderTokens::with_user_row_for(
                "owner-user",
                github_row("manual", "owner-token"),
            )),
            None,
            None,
            Arc::new(GitHubAppRegistry::default()),
        )
    }

    fn fake_db_pool() -> crate::db::DbPool {
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://fake:fake@localhost:1/fake")
            .unwrap()
    }

    fn binding_context(filter_user_id: Option<&str>) -> BindingContext {
        let now = chrono::Utc::now();
        BindingContext {
            binding_id: "tb_test".to_string(),
            trigger_id: None,
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
