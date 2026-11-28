//! Sandbox events the workspace sync publishes to the task, for what a
//! container log would lose with the pod.

use tracing::{error, warn};

use super::restic;
use crate::state::SandboxState;

/// Report that the restore left out a snapshot object it could not read.
///
/// Says only that one was skipped: its time is inside the object that will not
/// read, so whether the tree is behind what the task last stored is unknown.
pub(super) fn report_skipped_unreadable_snapshot(state: &SandboxState) {
    error!(
        task_id = %state.config.task_id,
        format = restic::FORMAT,
        "Skipped a stored workspace snapshot that would not read; restored the newest one that would"
    );
    publish_workspace_event(
        state,
        "workspace_snapshot_unreadable",
        "A stored workspace snapshot could not be read, so this pod restored the newest one it could read. If the unreadable snapshot was the newer one, work saved after the restored snapshot is not in this workspace.",
    );
}

/// Queue one sandbox event carrying `message` under `sub_type`.
pub(super) fn publish_workspace_event(state: &SandboxState, sub_type: &str, message: &str) {
    let payload = serde_json::json!({ "format": restic::FORMAT, "message": message });
    match serde_json::to_string(&payload) {
        Ok(payload_json) => {
            state.events.enqueue(sandbox_proto::PublishEventRequest {
                task_id: state.config.task_id.clone(),
                event_type: sandbox_proto::EventType::Sandbox as i32,
                sub_type: sub_type.to_string(),
                payload_json,
                event_id: String::new(),
                sequence: 0,
                run_id: String::new(),
            });
        }
        Err(e) => warn!(error = %e, sub_type, "Failed to serialize a workspace sync event"),
    }
}
