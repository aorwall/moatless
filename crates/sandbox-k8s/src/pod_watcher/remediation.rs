//! What the watcher does about a crash loop, and what a clean exit means for
//! the Deployment behind it.

use super::watch::WatcherDeps;
use super::*;

/// Publish a `server` `Failed` event for a named server.
pub(super) async fn publish_server_failed(
    event_publisher: &dyn EventPublisher,
    task_id: &str,
    server_name: &str,
    message: String,
) {
    event_publisher
        .publish_server_event(
            task_id,
            ServerSubType::Failed,
            &ServerEventPayload {
                server_name: server_name.to_string(),
                status: ServerSubType::Failed,
                message,
            },
        )
        .await;
}

/// Stop a crash-looping sandbox through the lifecycle (Stopped/Error intent +
/// graceful stop + scale-to-0).
///
/// Returns whether the stop landed, so callers can clear their pending-retry
/// flag; without a lifecycle handle (DB-less) there is no intent owner, so the
/// crash loop is left for the reconciler rather than scaled down directly.
pub(super) async fn crash_autostop(deps: &WatcherDeps, task_id: &str) -> bool {
    let Some(lifecycle) = deps.lifecycle.as_ref() else {
        debug!(task_id = %task_id, "No lifecycle handle — leaving crash-looping sandbox for the reconciler");
        return false;
    };
    match lifecycle.stop(task_id, SandboxStopReason::Error).await {
        Ok(_) => true,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Crash auto-stop via lifecycle failed; will retry");
            false
        }
    }
}

/// What to do with the Deployment when a sandbox pod exits cleanly (exit 0
/// after SIGTERM). See the call site for why a clean exit is ambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CleanExitAction {
    /// Intent is `Removed` — lifecycle already deleted the Deployment as part
    /// of the remove, so there is nothing left to scale (a patch would 404).
    AlreadyRemoved,
    /// Intent is `Stopped` — the SIGTERM is the pod draining after a deliberate
    /// stop. Confirm the scale-to-0.
    ConfirmStop,
    /// Intent is `Running`, or unknown — an involuntary node event SIGTERMed a
    /// pod the Deployment still wants. Leave it for K8s to reschedule.
    LeaveForReschedule,
}

/// Map durable Sandbox intent onto the clean-exit action.
///
/// `None` means intent couldn't be read (no lifecycle handle, missing Task row,
/// or a store error) and fails safe: a reschedule after an eviction is
/// recoverable, a wrongly cancelled active task is not.
pub(super) fn clean_exit_action_for_intent(
    desired: Option<SandboxDesiredState>,
) -> CleanExitAction {
    match desired {
        Some(SandboxDesiredState::Removed) => CleanExitAction::AlreadyRemoved,
        Some(SandboxDesiredState::Stopped) => CleanExitAction::ConfirmStop,
        Some(SandboxDesiredState::Running) | None => CleanExitAction::LeaveForReschedule,
    }
}

/// Decide what a clean sandbox exit means for the Deployment.
///
/// Intent is the authority: lifecycle alone can distinguish "Deployment deleted
/// by a remove" from "Deployment scaled to 0 by a stop" — the K8s API reports
/// both as zero desired replicas (`get_deployment_replicas` maps a 404 to `(0,
/// 0)`). Without a lifecycle handle (DB-less setups), falls back to the replica
/// count, which at least separates a stop from a still-wanted replica.
pub(super) async fn clean_exit_action(
    deps: &WatcherDeps,
    k8s: &K8sClient,
    task_id: &str,
) -> CleanExitAction {
    let Some(lifecycle) = deps.lifecycle.as_ref() else {
        return match deployment::get_deployment_replicas(k8s, task_id).await {
            Ok((0, _)) => CleanExitAction::ConfirmStop,
            Ok(_) => CleanExitAction::LeaveForReschedule,
            Err(e) => {
                warn!(
                    task_id = %task_id,
                    error = %e,
                    "Could not read deployment replicas on clean exit; leaving deployment untouched"
                );
                CleanExitAction::LeaveForReschedule
            }
        };
    };
    let desired = match lifecycle.desired_state(task_id).await {
        Ok(desired) => Some(desired),
        Err(e) => {
            debug!(
                task_id = %task_id,
                error = %e,
                "Could not read sandbox intent on clean exit; leaving deployment untouched"
            );
            None
        }
    };
    clean_exit_action_for_intent(desired)
}

/// Drive the attach-failure storage fallback for one wedged pod, publishing the
/// `storage_degraded` warning (mirroring the provision path) on a real rebuild.
///
/// The per-pod guard is set regardless of outcome, so a no-op or failed rebuild
/// waits for the next clean start rather than firing every sweep tick.
pub(super) async fn maybe_run_storage_fallback(
    event_publisher: &dyn EventPublisher,
    state: &mut PodObservedState,
    deps: &WatcherDeps,
) {
    let Some(fallback) = deps.storage_fallback.as_ref() else {
        return;
    };
    // The generation ties the rebuilt pod to the same operation. A pod our
    // provider built always carries the label; without it we cannot correlate
    // the replacement, so decline rather than start an unlabelled pod.
    let Some(generation) = state.operation_generation else {
        warn!(
            task_id = %state.task_id,
            pod_name = %state.pod_name,
            "attach failure on a pod with no operation generation; skipping storage fallback"
        );
        state.storage_fallback_requested = true;
        return;
    };

    warn!(
        task_id = %state.task_id,
        pod_name = %state.pod_name,
        "Pod stuck Pending on a volume-attach failure — attempting emptyDir + S3 fallback"
    );
    let outcome = fallback
        .rebuild_without_pvc(&state.task_id, generation)
        .await;
    state.storage_fallback_requested = true;

    if outcome == StorageFallbackOutcome::Rebuilt {
        event_publisher
            .publish_sandbox_event_with_payload(
                &state.task_id,
                SandboxStatus::Initializing,
                &SandboxEventPayload {
                    message: "Persistent storage could not attach; the workspace was \
                              restored from its last snapshot onto ephemeral storage and \
                              will not survive another restart until the volume recovers."
                        .to_string(),
                    lifecycle_event: Some("storage_degraded".to_string()),
                    error: Some("PVC attach failed; rebuilt on emptyDir from S3".to_string()),
                    ..Default::default()
                },
            )
            .await;
    }
}
