//! Sandbox-replacement ordering: fixed harness selection and restore-gated
//! readiness survive a real pod replacement (requires sandbox).
//!
//! Env gate: `SANDBOX_E2E` must be set to run. Deliberately credential-free — it
//! creates the task with **no** initial message, so no Claude/Codex run is
//! needed and it executes on the sandbox tier (no agent token).
//!
//! This is the compose-tier counterpart to the deterministic lib-tier proof
//! `init::replacement_test::restore::restore_precedes_the_harness_accepting_the_
//! resumed_turn`, which pins the fine-grained ordering (restored session state is
//! on disk before the harness accepts the resumed turn) against an in-memory
//! backend. That lib test cannot run the real `run_init` (fatal git clone, real
//! credential/settings writes, backend RPCs), so the two invariants only a *real*
//! replacement pod can show are asserted here:
//!
//!   1. **Fixed harness selection survives replacement.** The Task is pinned to a
//!      specific `agent_type` at creation (ADR-0004); after a real stop → start
//!      cycle rebuilds the pod through `build_harness`, the Task's fixed selection
//!      is unchanged — the replacement pod runs the same harness, never a silent
//!      default.
//!   2. **Restore gates readiness (restore precedes acceptance).** Production
//!      `run_init` runs `transcript_sync::download_on_start` (restore) inside
//!      `run_core_init`, *before* `open_for_work` flips `InitPhase::Ready`, emits
//!      `sandbox.ready`, and performs the one authoritative dispatch. External
//!      dispatch is a no-op until Ready. So a fresh post-replacement
//!      `sandbox.ready` event is observable evidence that restore completed before
//!      any new work is accepted, and workspace/session state persists across the
//!      replacement.

use std::time::Duration;

use integration_tests::{ensure_template_repo, sandbox_helpers, test_client};
use moatless_api_schemas::{
    sandbox::WriteFileRequestBody, shared::AgentType, task::CreateTaskRequest,
};
use moatless_cli::events::EventCollector;

#[tokio::test]
#[ignore] // Requires SANDBOX_E2E=1
async fn replacement_preserves_fixed_selection_and_restore_gated_readiness() {
    if std::env::var("SANDBOX_E2E").is_err() {
        eprintln!("Skipping: SANDBOX_E2E not set");
        return;
    }

    let client = test_client().await;
    let repo_id = ensure_template_repo(&client).await;

    let collector = EventCollector::start(&client)
        .await
        .expect("Failed to start event collector");

    // Pin an explicit, non-default agent type so "fixed selection survived" is a
    // real assertion and not a tautology against the backend default. No initial
    // message: this run needs no agent token, so it stays on the sandbox tier.
    let fixed_agent_type = AgentType("claude-code-cli".to_string());
    let req = CreateTaskRequest {
        repository_id: Some(repo_id.clone()),
        workspace_id: None,
        message: None,
        name: Some("Sandbox Replacement Ordering E2E".into()),
        branch: None,
        description: None,
        agent_mode: None,
        agent_type: Some(fixed_agent_type.clone()),
        contexts: vec![],
        file_ids: vec![],
        model: None,
        headless: None,
        parent_task_id: None,
    };
    let created = client.api().create_task(&req).await.unwrap();
    let task_id = &created.task_id;

    // Sandbox comes up (its first run_init completes restore before Ready).
    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "Sandbox did not become ready"
    );

    // The fixed selection is in effect before any replacement.
    let before = client.api().get_task(task_id).await.unwrap();
    assert_eq!(
        before.agent_type.as_str(),
        fixed_agent_type.as_str(),
        "Task should keep the fixed agent_type it was created with"
    );

    // Marker file to confirm workspace state survives the replacement.
    client
        .api()
        .write_file(
            task_id,
            &WriteFileRequestBody {
                path: "replacement-marker.txt".into(),
                content: "before replacement".into(),
            },
        )
        .await
        .unwrap();

    let ready_before = collector.count_events("sandbox", "ready", task_id);

    // Stop → drains the pod (sync-on-shutdown uploads session state).
    client.api().stop_sandbox(task_id).await.unwrap();
    assert!(
        sandbox_helpers::wait_for_sandbox_status(
            &client,
            task_id,
            "stopped",
            Duration::from_secs(60)
        )
        .await,
        "Sandbox did not reach stopped state"
    );

    // Start → a real replacement pod runs a fresh run_init: download_on_start
    // (restore) → open_for_work (flip Ready, emit sandbox.ready, dispatch).
    client.api().start_task(task_id).await.unwrap();
    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "Sandbox did not become ready after replacement"
    );

    // (2) A fresh post-replacement `sandbox.ready` — the gate that follows restore
    // in run_init and precedes any acceptance of new work.
    let ready_after = collector
        .wait_for_nth_event(
            "sandbox",
            "ready",
            task_id,
            ready_before + 1,
            Duration::from_secs(30),
        )
        .await;
    assert!(
        ready_after.is_some(),
        "Expected a fresh sandbox.ready after replacement (restore-gated readiness), \
         had {ready_before} before"
    );

    // Workspace/session state survived the replacement (restore landed).
    let readback = client
        .api()
        .read_file(task_id, "replacement-marker.txt")
        .await
        .unwrap();
    assert_eq!(
        readback.content, "before replacement",
        "Marker file must persist across the pod replacement"
    );

    // (1) The fixed harness selection is unchanged after the real replacement.
    let after = client.api().get_task(task_id).await.unwrap();
    assert_eq!(
        after.agent_type.as_str(),
        fixed_agent_type.as_str(),
        "Fixed agent_type must survive a pod replacement (no silent default)"
    );

    collector.stop();
    let _ = client.api().delete_task(task_id).await;
}
