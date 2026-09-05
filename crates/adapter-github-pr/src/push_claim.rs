//! The claim a Task files on a sha it pushed.
//!
//! Every other GitHub event a Task produces names its author: a comment carries
//! the `Moatless task:` footer, and the typed `moat gh` routes record the id
//! GitHub allocated. A `git push` passes through neither, so nothing named the
//! Task behind a `pull_request.synchronize` event and the push echoed back into
//! the Task that made it.
//!
//! The sandbox files this claim as the push completes
//! (`init::git_push_watcher`), before GitHub's webhook for the same sha
//! arrives. Routing reads it back through [`external_id::aliases`] and passes
//! over the Task the claim names.

use moatless_ports::task::{ExternalEvent, TranscriptAppend};
use moatless_types::store::StoreError;

use super::external_id;
use super::routing::ADAPTER_LABEL;

/// Record `sha` as `task_id`'s own push into `owner/repo`.
///
/// Idempotent: a redelivered claim for a sha already held rewrites the same row.
pub async fn record(
    messages: &dyn TranscriptAppend,
    task_id: &str,
    owner: &str,
    repo: &str,
    sha: &str,
) -> Result<(), StoreError> {
    messages
        .record_external_event(
            task_id,
            &ExternalEvent {
                external_id: external_id::push(owner, repo, sha),
                provider: ADAPTER_LABEL.to_string(),
                source_name: Some(format!("{owner}/{repo}")),
            },
        )
        .await
}
