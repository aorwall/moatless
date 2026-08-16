//! Outbound reply capabilities for GitHub pull requests.

use moatless_identity::git_host_repo::GitHostRepo;
use moatless_loops::adapters::error::{AdapterError, SharedAdapterError};
use moatless_loops::adapters::models::{
    AdapterConnection, BindingContext, LoopContext, OutboundReply, SubscriptionMatch, TaskBinding,
};
use moatless_loops::adapters::{AdapterBindingReply, AdapterMessageReply};

use super::adapter::GithubPrAdapter;
use super::external_id::{self, CommentTarget};
use super::subject::GithubPrSubject;

#[async_trait::async_trait]
impl AdapterBindingReply for GithubPrAdapter {
    /// Legacy thread-based outbound is not supported for GitHub. Outbound
    /// goes through `send_reply_for_binding` instead.
    async fn send_reply(
        &self,
        _thread: &BindingContext,
        _config: &LoopContext,
        _message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        Err(SharedAdapterError::BadRequest(
            "GitHub provider uses binding-based outbound; \
             call send_reply_for_binding"
                .into(),
        )
        .into())
    }

    async fn send_reply_for_binding(
        &self,
        binding: &TaskBinding,
        _connection: &AdapterConnection,
        rule: Option<&SubscriptionMatch>,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let subject = GithubPrSubject::from_binding(binding)?;
        let api = self
            .auth()
            .client_for_outbound_reply(
                message.author_user_id.as_deref(),
                binding,
                rule,
                &GitHostRepo::github(subject.owner(), subject.repo()),
            )
            .await?;

        // The `Moatless task:` footer (carrying the task URL) doubles as the
        // echo-filter source on the way back in — no hidden marker needed.
        let mut body = message.text.clone();
        if let Some(ref footer) = message.footer {
            body.push_str(footer);
        }

        let comment_id = api
            .post_comment(subject.owner(), subject.repo(), subject.pr_number(), &body)
            .await?;
        Ok(Some(external_id::issue_comment(comment_id)))
    }
}

#[async_trait::async_trait]
impl AdapterMessageReply for GithubPrAdapter {
    async fn reply_to_message(
        &self,
        binding: &TaskBinding,
        connection: &AdapterConnection,
        rule: Option<&SubscriptionMatch>,
        target_external_id: &str,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        // Only PR review comments support threaded replies. Top-level issue
        // comments fall back to the binding-level reply (a new top-level
        // PR comment).
        match external_id::comment_target_from_id(target_external_id) {
            Some(CommentTarget::Review(comment_id)) => {
                let subject = GithubPrSubject::from_binding(binding)?;
                let api = self
                    .auth()
                    .client_for_outbound_reply(
                        message.author_user_id.as_deref(),
                        binding,
                        rule,
                        &GitHostRepo::github(subject.owner(), subject.repo()),
                    )
                    .await?;

                // The `Moatless task:` footer doubles as the echo-filter source.
                let mut body = message.text.clone();
                if let Some(ref footer) = message.footer {
                    body.push_str(footer);
                }

                let new_id = api
                    .reply_to_review_comment(
                        subject.owner(),
                        subject.repo(),
                        subject.pr_number(),
                        comment_id,
                        &body,
                    )
                    .await?;
                Ok(Some(external_id::review_comment(new_id)))
            }
            _ => {
                self.send_reply_for_binding(binding, connection, rule, message)
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::{Method, StatusCode};

    use super::super::test_support::{StubResponse, start_github_api_stub};
    use super::*;
    use moatless_identity::auth::test_support::{StubProviderTokens, github_row};
    use moatless_loops::adapters::models::{AdapterConnection, OutboundReply, TaskBinding};
    use moatless_loops::adapters::{AdapterBindingReply, AdapterMessageReply};

    #[tokio::test]
    async fn outbound_capabilities_keep_exact_github_request_budgets() {
        let stub = start_github_api_stub(|method, path, uri| match (method, path) {
            (Method::POST, "/repos/soaplabs/repo/issues/42/comments") => {
                StubResponse::json(StatusCode::CREATED, serde_json::json!({"id": 101}))
            }
            (Method::POST, "/repos/soaplabs/repo/pulls/42/comments/55/replies") => {
                StubResponse::json(StatusCode::CREATED, serde_json::json!({"id": 102}))
            }
            _ => StubResponse::not_found(uri),
        })
        .await;

        let provider = provider(stub.base_url());
        let binding = binding_target(Some("owner-user"));
        let connection = connection();

        assert_eq!(
            provider
                .send_reply_for_binding(&binding, &connection, None, &outbound_reply())
                .await
                .unwrap()
                .as_deref(),
            Some("github:ic:101")
        );
        assert_eq!(
            provider
                .reply_to_message(
                    &binding,
                    &connection,
                    None,
                    "github:rc:55",
                    &outbound_reply(),
                )
                .await
                .unwrap()
                .as_deref(),
            Some("github:rc:102")
        );

        // Every request below was provoked by this test: the stub is reached
        // only by the client this Adapter built against it.
        let recorded = stub.recorded_requests().await;
        let selected_outbound_token = Some("Bearer owner-token");
        let outbound_requests: Vec<_> = recorded.iter().collect();
        let observed: Vec<(&str, &str)> = outbound_requests
            .iter()
            .map(|request| (request.method.as_str(), request.uri.as_str()))
            .collect();
        assert_eq!(
            observed,
            vec![
                ("POST", "/repos/soaplabs/repo/issues/42/comments"),
                ("POST", "/repos/soaplabs/repo/pulls/42/comments/55/replies"),
            ]
        );
        assert!(
            outbound_requests
                .iter()
                .all(|request| request.authorization.as_deref() == selected_outbound_token),
            "outbound replies keep their selected user token: {recorded:?}"
        );
    }

    fn provider(api_base: &str) -> GithubPrAdapter {
        GithubPrAdapter::new(
            None,
            "https://app.example.com".to_string(),
            moatless_identity::git_host_tokens::seam_over(Arc::new(
                StubProviderTokens::with_user_row_for(
                    "owner-user",
                    github_row("manual", "owner-token"),
                ),
            )),
        )
        .with_api_base(api_base)
    }

    fn binding_target(filter_user_id: Option<&str>) -> TaskBinding {
        let now = chrono::Utc::now();
        TaskBinding {
            id: "tb_test".into(),
            task_id: "task_test".into(),
            adapter_kind: "github_pr".into(),
            subject_kind: "github_pr".into(),
            subject_key: "soaplabs/repo#42".into(),
            connection_id: "pc_test".into(),
            loop_id: None,
            destination_config: serde_json::json!({
                "owner": "soaplabs",
                "repo": "repo",
                "pr_number": 42,
                "filter_user_id": filter_user_id,
            }),
            metadata: serde_json::json!({"filter_user_id": filter_user_id}),
            last_external_event_id: None,
            origin: "connect".into(),
            created_at: now,
            last_event_at: now,
            external_status: None,
            external_state_kind: None,
            external_status_raw: None,
            external_review_decision: None,
            external_mergeable_state: None,
            external_status_updated_at: None,
            active: true,
            deactivated_at: None,
        }
    }

    fn connection() -> AdapterConnection {
        let now = chrono::Utc::now();
        AdapterConnection {
            id: "pc_test".to_string(),
            adapter_kind: "github".to_string(),
            connection_kind: "org".to_string(),
            scope_value: "soaplabs".to_string(),
            credential_ref: None,
            webhook_secret: None,
            transport_config: serde_json::json!({}),
            created_by: "owner-user".to_string(),
            scope: moatless_backend_core::scope::Scope::User,
            owner_user_id: Some("owner-user".to_string()),
            created_at: now,
            updated_at: now,
        }
    }

    fn outbound_reply() -> OutboundReply {
        OutboundReply {
            text: "Characterization reply".to_string(),
            task_url: None,
            footer: Some(
                "\n\n---\nMoatless task: https://app.example.com/tasks/task_test".to_string(),
            ),
            author_user_id: Some("owner-user".to_string()),
            is_error: false,
            files: Vec::new(),
        }
    }
}
