//! Startup dispatch in a shared sandbox: every Member's pending Messages reach
//! that Member's own agent, each exactly once.

use std::sync::Arc;

use super::backend::ReplayBackend;
use super::support::{
    add_recording_member, recorded_inputs, recording_state, settled_inputs, text_message,
};
use crate::init::{InitPhase, WorkspaceOrigin, open_for_work};

/// A pod replaced while two Members each had a Message pending: neither was
/// delivered by the pod that died, so the replacement's startup dispatch owes
/// both, and a Member's Message must not reach the other Member's agent.
#[tokio::test]
async fn startup_delivers_each_members_pending_message_once_to_its_own_agent() {
    let backend = ReplayBackend::new(Ok(vec![]), None);
    let (state, boot_recorder) = recording_state(
        Some("claude-code"),
        InitPhase::Starting,
        Arc::clone(&backend) as _,
    );
    let boot_task = state.config.task_id.clone();
    let joined_recorder = add_recording_member(&state, "joined-task").await;
    backend
        .push_member_pending(&boot_task, text_message("for the boot member", "u-boot"))
        .await;
    backend
        .push_member_pending(
            "joined-task",
            text_message("for the joined member", "u-joined"),
        )
        .await;

    open_for_work(&state, WorkspaceOrigin::Persisted).await;

    let boot_inputs = recorded_inputs(&boot_recorder, 1).await;
    let joined_inputs = recorded_inputs(&joined_recorder, 1).await;
    assert_eq!(
        boot_inputs
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>(),
        vec!["for the boot member"],
        "the boot Member's agent must receive its own Message and nothing else"
    );
    assert_eq!(
        joined_inputs
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>(),
        vec!["for the joined member"],
        "the joined Member's agent must receive its own Message and nothing else"
    );

    assert_eq!(task_id_of(&boot_inputs[0]), Some(boot_task.as_str()));
    assert_eq!(task_id_of(&joined_inputs[0]), Some("joined-task"));

    // A notification racing startup finds both rows acknowledged, so a second
    // dispatch pass delivers neither again.
    for task_id in [boot_task.as_str(), "joined-task"] {
        crate::agent::dispatcher::dispatch_pending_messages_for_task(
            state.clone(),
            task_id.to_string(),
        )
        .await;
    }
    assert_eq!(settled_inputs(&boot_recorder).await.len(), 1);
    assert_eq!(settled_inputs(&joined_recorder).await.len(), 1);
    assert!(backend.member_pending(&boot_task).await.is_empty());
    assert!(backend.member_pending("joined-task").await.is_empty());
}

/// The `TASK_ID` the harness would stamp on the Turn's subprocess.
fn task_id_of(input: &crate::agent::harness::Input) -> Option<&str> {
    input.env.as_ref()?.get("TASK_ID").map(String::as_str)
}
