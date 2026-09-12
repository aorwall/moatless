//! What a turn carries into the sink: the id the client minted, the skills its
//! text names, and the listing position the reply reports.

use super::fixtures::*;
use super::*;

#[tokio::test]
async fn a_turn_persists_under_the_id_the_client_minted() {
    let (result, calls) = run(turn(None)).await;
    assert!(result.is_ok());
    assert_eq!(
        calls,
        vec![Call::SendTurn {
            task_id: "task-1".to_string(),
            message_uuid: "msg-1".to_string(),
            model: None,
            agent_mode: Some("edit".to_string()),
            effort: None,
            attachments: vec![],
            skills: vec![],
        }]
    );
}

/// A turn whose prompt reads the way the composer leaves it after a pick.
fn turn_saying(text: &str) -> Value {
    let mut command = turn(None);
    command["message"]["text"] = json!(text);
    command
}

/// The `$name` stays in the text, because that is what the T3 timeline
/// renders as a chip. Moatless frontloads by name instead, so the same pick
/// has to reach `skills` as well or the prompt is the only thing that ships.
#[tokio::test]
async fn a_turn_activates_the_skill_its_text_mentions() {
    let (result, calls) = run(turn_saying("$git-workflow open the PR")).await;
    assert!(result.is_ok());
    assert!(
        matches!(calls.as_slice(), [Call::SendTurn { skills, .. }] if skills == &["git-workflow"]),
        "expected git-workflow activated, got {calls:?}"
    );
}

/// A mention only counts when the Workspace offers it, so an env var in
/// prose stays prose.
#[tokio::test]
async fn a_mention_of_something_that_is_not_a_skill_activates_nothing() {
    let (_, calls) = run(turn_saying("echo $HOME and $linearity")).await;
    assert!(
        matches!(calls.as_slice(), [Call::SendTurn { skills, .. }] if skills.is_empty()),
        "expected no skills, got {calls:?}"
    );
}

/// The create-thread composer is the one the person picks from most, and it
/// sends its pick on the bootstrapped turn that creates the Task.
#[tokio::test]
async fn a_bootstrapped_turn_activates_the_skills_it_mentions() {
    let mut command = turn_saying("$linear then $git-workflow");
    command["bootstrap"] = json!({
        "createThread": {
            "projectId": "ws-1",
            "title": "New chat",
            "runtimeMode": "full-access",
            "interactionMode": "default",
            "branch": null,
            "worktreePath": null,
            "createdAt": "2026-07-01T09:00:00.000Z"
        }
    });
    let (result, calls) = run_against(command, None).await;
    assert!(result.is_ok());
    assert!(
        matches!(
            calls.as_slice(),
            [Call::CreateThread { skills, .. }] if skills == &["linear", "git-workflow"]
        ),
        "expected both picks in the order they were made, got {calls:?}"
    );
}

#[tokio::test]
async fn a_successful_command_reports_the_listing_position() {
    let (result, _) = run(turn(None)).await;
    assert_eq!(result.unwrap(), json!({ "sequence": 42 }));
}
