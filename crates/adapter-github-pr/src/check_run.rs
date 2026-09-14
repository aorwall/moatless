//! Durable completion of a failed check run's notice.
//!
//! Normalization composes the notice and stops there: finishing it takes the
//! second read [`super::api::CheckAnnotation`] describes, which a webhook
//! handler must not block on.

use moatless_workspace_types::repository::GitHostRepo;
use tracing::warn;

use moatless_adapter_model::error::AdapterError;
use moatless_types::access_control::Actor;
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::InboundMessage;
use moatless_types::loops::SubscriptionMatch;

use super::api::CheckAnnotation;
use super::router::GithubPrInboundRouter;
use super::text::check_run_annotations_text;

/// Facts persisted for one durable check-run notice.
///
/// Carries the composed message rather than the raw payload: normalization owns
/// the wording and the sender's resolved user, and this execution only appends
/// what it had to fetch.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Execution {
    pub(super) owner: String,
    pub(super) repo: String,
    pub(super) check_run_id: u64,
    /// `output.annotations_count` from the webhook. Zero skips the fetch, which
    /// is every check run whose app writes a real `output.summary` instead.
    pub(super) annotations_count: u64,
    pub(super) message: InboundMessage,
}

impl GithubPrInboundRouter {
    /// Deliver a failed check run's notice, completed with the annotations the
    /// webhook only counted.
    ///
    /// A failed read degrades to the bare notice rather than failing the
    /// delivery: the annotations say what broke, but dropping the notice with
    /// them leaves the task believing CI passed.
    pub(super) async fn run_check_run(
        &self,
        connection: &AdapterConnection,
        trigger: Option<&SubscriptionMatch>,
        execution: &Execution,
    ) -> Result<(), AdapterError> {
        let mut message = execution.message.clone();

        // A payload without `check_run.id` normalizes to id 0, which names no
        // check run: skip a read that can only 404.
        if execution.annotations_count > 0 && execution.check_run_id > 0 {
            match self.check_run_annotations(execution).await {
                Ok(annotations) => {
                    message
                        .text
                        .push_str(&check_run_annotations_text(&annotations));
                }
                Err(error) => warn!(
                    owner = %execution.owner,
                    repo = %execution.repo,
                    check_run_id = execution.check_run_id,
                    %error,
                    "Could not read check-run annotations; delivering the notice without them"
                ),
            }
        }

        self.route_event(connection, trigger, message).await
    }

    /// Read the annotations as the same user the notice is attributed to, which
    /// normalization already resolved from the webhook's sender.
    async fn check_run_annotations(
        &self,
        execution: &Execution,
    ) -> Result<Vec<CheckAnnotation>, AdapterError> {
        let Some(adapter) = self.adapter.as_ref() else {
            return Ok(Vec::new());
        };
        adapter
            .auth()
            .client_for(
                &Actor::of(&execution.message.user_id),
                &GitHostRepo::github(&execution.owner, &execution.repo),
            )
            .await?
            .get_check_run_annotations(&execution.owner, &execution.repo, execution.check_run_id)
            .await
    }
}
