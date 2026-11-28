//! One watcher event: the transitions it detects and what each publishes.

use super::diagnostics::{emit_turn_interrupted, fetch_sandbox_logs};
use super::observed::{extract_task_id, format_sandbox_lifecycle_message, record_sandbox_startup};
use super::remediation::{CleanExitAction, clean_exit_action, crash_autostop};
use super::watch::WatcherDeps;
use super::*;

/// Exit code used by the sandbox to signal a permanent init failure (sysexits
/// `EX_CONFIG`).
pub(super) const PERMANENT_FAILURE_EXIT_CODE: i32 = 78;

/// Return sibling pod states for a given `task_id`, excluding a specific pod.
pub(super) fn sibling_pods_for_task<'a>(
    pod_states: &'a HashMap<String, PodObservedState>,
    task_id: &str,
    exclude_pod: &str,
) -> Vec<&'a PodObservedState> {
    pod_states
        .values()
        .filter(|s| s.task_id == task_id && s.pod_name != exclude_pod)
        .collect()
}

/// Process a single watcher event and detect state transitions.
#[expect(
    clippy::cognitive_complexity,
    reason = "Kubernetes watch adapter still coordinates Sandbox observation, publication, and cleanup intent until the Sandbox observation module lands"
)]
pub(super) async fn process_watcher_event(
    k8s: &K8sClient,
    event: &watcher::Event<Pod>,
    pod_states: &mut HashMap<String, PodObservedState>,
    event_publisher: &dyn EventPublisher,
    max_crash_restart_count: i32,
    deps: &WatcherDeps,
) {
    match event {
        watcher::Event::Apply(pod) | watcher::Event::InitApply(pod) => {
            let is_init = matches!(event, watcher::Event::InitApply(_));
            let task_id = if let Some(id) = extract_task_id(pod) {
                id
            } else {
                let name = pod.metadata.name.as_deref().unwrap_or("unknown");
                warn!(
                    pod_name = name,
                    "Pod has no moatless.io/task-id label — skipping"
                );
                return;
            };
            let pod_name = pod
                .metadata
                .name
                .as_deref()
                .unwrap_or("unknown")
                .to_string();

            let mut new_state = extract_pod_observed_state(pod, &pod_name, &task_id);

            // Node-backed workspace/Docker archives outlive the pod. Record a
            // scheduled location on first observation and whenever the node or
            // operation generation changes. Watcher persistence is best-effort;
            // start/remove also synchronously merge the current pod before a
            // load-bearing placement or deletion decision.
            let should_record_node = deps.node_archive_enabled
                && new_state.node_name.is_some()
                && pod_states.get(&pod_name).is_none_or(|previous| {
                    previous.node_name != new_state.node_name
                        || previous.operation_generation != new_state.operation_generation
                });
            if should_record_node
                && let Err(error) =
                    crate::node_archive::record_observed_pod(k8s, &task_id, pod).await
            {
                warn!(
                    task_id,
                    pod_name,
                    error = %error,
                    "Failed to record node-local sandbox archive location"
                );
            }

            // During InitApply, populate the map silently without publishing events
            if is_init {
                if new_state.phase == "Failed" {
                    // Corpse predating this watcher — start its GC clock now.
                    new_state.failed_since_ms =
                        Some(k8s_openapi::jiff::Timestamp::now().as_millisecond());
                }
                pod_states.insert(pod_name, new_state);
                return;
            }

            let prev_state = pod_states.get(&pod_name);
            let is_new_pod = prev_state.is_none();

            // Cross-pod recovery: new pod appearing while a sibling is in Failed/Unknown
            if is_new_pod {
                let siblings = sibling_pods_for_task(pod_states, &task_id, &pod_name);
                let failed_siblings = siblings
                    .iter()
                    .filter(|s| matches!(s.phase.as_str(), "Failed" | "Unknown"));
                let has_failed_sibling = failed_siblings.clone().next().is_some();
                // Also inherit from a failed replacement: the pod that published
                // `fatal` may already be deleted or garbage collected.
                new_state.replaces_published_failure = failed_siblings
                    .clone()
                    .any(|s| s.fatal_published || s.replaces_published_failure);
                // A failed replacement already announced this recovery.
                let recovery_announced = failed_siblings
                    .clone()
                    .any(|s| s.replaces_published_failure);

                if has_failed_sibling && !recovery_announced {
                    info!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        "New pod replacing failed sibling — emitting pod_recovering"
                    );
                    event_publisher
                        .publish_sandbox_event_with_payload(
                            &task_id,
                            SandboxStatus::Initializing,
                            &SandboxEventPayload {
                                message: "Recovering from a failed pod".to_string(),
                                lifecycle_event: Some("pod_recovering".to_string()),
                                ..Default::default()
                            },
                        )
                        .await;
                }
            }

            // Carried across Apply events so a flapping container (one watch
            // event per restart) publishes each status once, not once per Apply.
            new_state.last_published_sandbox_status =
                prev_state.and_then(|p| p.last_published_sandbox_status);
            if let Some(prev) = prev_state {
                new_state.replaces_published_failure = prev.replaces_published_failure;
            }

            let initial_plan = PodObservationPolicy::evaluate(
                prev_state,
                &new_state,
                ObservationContext::default(),
            );

            // Check if this is an intentional stop (Deployment scaled to 0).
            // When the idle reaper or user explicitly stops a sandbox, the pod
            // terminates with exit code 137 (SIGKILL) which would otherwise be
            // misinterpreted as a crash.
            let is_intentional_stop = if initial_plan.has_error_transition {
                match deployment::get_deployment_replicas(k8s, &task_id).await {
                    Ok((desired, _)) => desired == 0,
                    Err(e) => {
                        debug!(task_id = %task_id, error = %e, "Could not check deployment replicas");
                        false
                    }
                }
            } else {
                false
            };
            let plan = if is_intentional_stop {
                PodObservationPolicy::evaluate(
                    prev_state,
                    &new_state,
                    ObservationContext {
                        intentional_stop: true,
                    },
                )
            } else {
                initial_plan
            };

            for transition in &plan.sandbox_transitions {
                let sub_type = &transition.sub_type;
                let error_msg = &transition.error;
                let sandbox_status = transition.status;

                if transition.is_error_event && !is_intentional_stop {
                    // A non-intentional error-level termination is a real failure.
                    // Count it (labelled for OOM-vs-crash slicing) and log at warn!
                    // so it surfaces in alerting, not buried in info noise.
                    metrics::counter!(
                        "sandbox_container_terminations_total",
                        "lifecycle_event" => sub_type.clone(),
                        "reason" => new_state
                            .effective_terminated_reason()
                            .unwrap_or_else(|| "unknown".to_string()),
                        "exit_code" => new_state
                            .effective_exit_code()
                            .map_or_else(|| "unknown".to_string(), |c| c.to_string()),
                    )
                    .increment(1);
                    warn!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        lifecycle_event = %sub_type,
                        sandbox_status = %sandbox_status,
                        error = ?error_msg,
                        exit_code = ?new_state.effective_exit_code(),
                        terminated_reason = ?new_state.effective_terminated_reason(),
                        failure_detail = ?new_state.failure_detail,
                        "Sandbox container terminated unexpectedly"
                    );
                } else {
                    info!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        lifecycle_event = %sub_type,
                        sandbox_status = %sandbox_status,
                        error = ?error_msg,
                        exit_code = ?new_state.exit_code,
                        failure_detail = ?new_state.failure_detail,
                        intentional_stop = is_intentional_stop,
                        "Pod lifecycle event detected"
                    );
                }

                // Metrics and logs above still record every detected edge —
                // only the event publish is deduplicated.
                if transition.skip_duplicate_publish
                    || (transition.is_error_event && new_state.replaces_published_failure)
                {
                    debug!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        lifecycle_event = %sub_type,
                        sandbox_status = %sandbox_status,
                        "Skipping duplicate sandbox status publish"
                    );
                    continue;
                }

                let mut event_generation = new_state.operation_generation;
                if let Some(observed_transition) = transition.lifecycle_observation {
                    let decision = classify_observation(
                        deps.lifecycle.as_ref(),
                        &task_id,
                        observed_transition,
                        new_state.operation_generation,
                    )
                    .await;
                    if !decision.publishes {
                        debug!(
                            task_id = %task_id,
                            pod_name = %pod_name,
                            lifecycle_event = %sub_type,
                            "Suppressing lifecycle-correlated pod observation"
                        );
                        continue;
                    }
                    event_generation = decision.event_generation;
                }

                // Involuntary eviction is inherently node-initiated — tag it so
                // the frontend can render an amber "recovering" chip. Other
                // intentional-stop initiators are set at their explicit publish
                // sites (reaper/handlers/provider), which the watcher can't see.
                let initiator = if sub_type == "pod_evicted" && !is_intentional_stop {
                    Some("eviction".to_string())
                } else {
                    None
                };

                // Time spent Pending before scheduling, surfaced on the edge event.
                let pending_seconds = if sub_type == "pod_scheduled" {
                    new_state.created_at_ms.map(|created| {
                        let now = k8s_openapi::jiff::Timestamp::now().as_millisecond();
                        (now - created).max(0) / 1000
                    })
                } else {
                    None
                };

                publish_sandbox_observation(
                    event_publisher,
                    &task_id,
                    sandbox_status,
                    &SandboxEventPayload {
                        message: format_sandbox_lifecycle_message(sub_type, error_msg.as_deref()),
                        lifecycle_event: Some(sub_type.clone()),
                        error: if is_intentional_stop {
                            None
                        } else {
                            error_msg.clone()
                        },
                        // Carry exit code/reason so consumers tell OOM (137)
                        // from a crash (1) without scraping pod state. Set
                        // even on intentional stops: it's diagnostic, not an
                        // error signal.
                        exit_code: new_state.effective_exit_code(),
                        terminated_reason: new_state.effective_terminated_reason(),
                        // Node-level correlation + disruption cause on every event.
                        node_name: new_state.node_name.clone(),
                        disruption_reason: new_state.disruption_reason.clone(),
                        initiator,
                        pending_seconds,
                    },
                    event_generation,
                )
                .await;
                new_state.last_published_sandbox_status = Some(sandbox_status);
            }

            // Carry over fatal_published from previous state, then reset on recovery.
            // startup_recorded is sticky (never reset) so re-readies aren't re-timed.
            if let Some(prev) = prev_state {
                new_state.fatal_published = prev.fatal_published;
                new_state.startup_recorded = prev.startup_recorded;
                // Sticky fire-once guards for the threshold sweep, so re-observing
                // a still-stuck pod doesn't re-emit its blocked/slow event.
                new_state.scheduling_blocked_published = prev.scheduling_blocked_published;
                new_state.startup_slow_published = prev.startup_slow_published;
                new_state.storage_fallback_requested = prev.storage_fallback_requested;
                // Carry a still-pending auto-stop so a failed scale-to-0 is
                // retried on this event (see the auto-stop block below).
                new_state.auto_stop_pending = prev.auto_stop_pending;
            }
            // Start (or carry over) the GC clock for pods in the terminal
            // Failed phase — K8s never removes them on its own.
            if new_state.phase == "Failed" {
                new_state.failed_since_ms = prev_state
                    .and_then(|p| p.failed_since_ms)
                    .or_else(|| Some(k8s_openapi::jiff::Timestamp::now().as_millisecond()));
                new_state.gc_requested = prev_state.is_some_and(|p| p.gc_requested);
            }
            let became_ready = plan
                .sandbox_transitions
                .iter()
                .any(|transition| transition.sub_type == "pod_ready");
            if became_ready {
                new_state.fatal_published = false;
                new_state.replaces_published_failure = false;
                // Time pod-creation → first Ready once per pod.
                if plan.should_record_startup {
                    record_sandbox_startup(pod);
                    new_state.startup_recorded = true;
                }
            }

            // Fatal means pod-level failure that K8s will NOT restart.
            // Container-level crashes (OOM, crash loop) are not fatal — K8s
            // restarts those; the auto-stop threshold handles persistent
            // crash loops separately.
            if plan.has_fatal_transition
                && !is_intentional_stop
                && !new_state.fatal_published
                && !new_state.replaces_published_failure
            {
                let error_msg = plan
                    .sandbox_transitions
                    .iter()
                    .find_map(|transition| transition.error.as_ref())
                    .cloned()
                    .unwrap_or_else(|| "Sandbox terminated unexpectedly".to_string());

                let exit_code = new_state.effective_exit_code();
                let terminated_reason = new_state.effective_terminated_reason();

                // Fetch logs on every fatal failure: narrowing to exit-78 and
                // the crash-restart threshold would leave a one-off failure
                // (e.g. "Error" mid-turn) with no trace.
                let logs = fetch_sandbox_logs(k8s, &pod_name, 50).await;

                metrics::counter!(
                    "sandbox_container_terminations_total",
                    "lifecycle_event" => "fatal".to_string(),
                    "reason" => terminated_reason
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string()),
                    "exit_code" => exit_code
                        .map_or_else(|| "unknown".to_string(), |c| c.to_string()),
                )
                .increment(1);
                warn!(
                    task_id = %task_id,
                    pod_name = %pod_name,
                    exit_code = ?exit_code,
                    terminated_reason = ?terminated_reason,
                    failure_detail = ?new_state.failure_detail,
                    "Sandbox pod failed irrecoverably"
                );

                let mut parts = vec![format!("Sandbox terminated: {error_msg}")];
                if let Some(code) = exit_code {
                    parts.push(format!("Exit code: `{code}`"));
                }
                if let Some(ref reason) = terminated_reason {
                    parts.push(format!("Reason: `{reason}`"));
                }
                // Name the node-level cause (spot reclaim, drain, preemption) in
                // the message itself, not just the payload chip, so the root cause
                // is legible wherever the message is read.
                if let Some(ref disruption) = new_state.disruption_reason {
                    parts.push(format!("Disruption: `{disruption}`"));
                }
                if let Some(ref logs) = logs {
                    parts.push(format!("Container logs:\n```\n{logs}\n```"));
                }
                let detailed_message = parts.join("\n");

                event_publisher
                    .publish_sandbox_event_with_payload(
                        &task_id,
                        SandboxStatus::Error,
                        &SandboxEventPayload {
                            message: detailed_message.clone(),
                            lifecycle_event: Some("fatal".to_string()),
                            error: Some(detailed_message),
                            exit_code,
                            terminated_reason,
                            node_name: new_state.node_name.clone(),
                            disruption_reason: new_state.disruption_reason.clone(),
                            ..Default::default()
                        },
                    )
                    .await;

                // Surface the interruption immediately as a taskTurn fact (with
                // cause) instead of waiting up to ~5 min for the reaper's
                // delayed `reconciled_orphaned_turn`.
                let cause = new_state
                    .disruption_reason
                    .clone()
                    .or_else(|| new_state.effective_terminated_reason());
                emit_turn_interrupted(event_publisher, &task_id, cause.as_deref()).await;

                new_state.fatal_published = true;
            }

            // Immediate fail for permanent init errors.
            // Exit code 78 signals the sandbox detected an unrecoverable failure
            // (repo not found, auth error, etc.) — no point retrying.
            let has_terminated = plan
                .sandbox_transitions
                .iter()
                .any(|transition| transition.sub_type == "container_terminated");
            if has_terminated
                && !is_intentional_stop
                && new_state.exit_code == Some(PERMANENT_FAILURE_EXIT_CODE)
            {
                warn!(
                    task_id = %task_id,
                    pod_name = %pod_name,
                    exit_code = ?new_state.exit_code,
                    failure_detail = ?new_state.failure_detail,
                    "Permanent init failure (exit 78), auto-stopping immediately"
                );

                let logs = fetch_sandbox_logs(k8s, &pod_name, 50).await;

                let mut error_parts = vec!["Sandbox stopped: permanent init failure.".to_string()];
                if let Some(ref detail) = new_state.failure_detail {
                    error_parts.push(format!("Error: `{detail}`"));
                }
                if let Some(ref logs) = logs {
                    error_parts.push(format!("Container logs:\n```\n{logs}\n```"));
                }
                let detailed_error = error_parts.join("\n");

                event_publisher
                    .publish_sandbox_event_with_payload(
                        &task_id,
                        SandboxStatus::Error,
                        &SandboxEventPayload {
                            message: detailed_error.clone(),
                            lifecycle_event: Some("auto_stopped".to_string()),
                            error: Some(detailed_error),
                            exit_code: new_state.effective_exit_code(),
                            terminated_reason: new_state.effective_terminated_reason(),
                            initiator: Some("crash_autostop".to_string()),
                            node_name: new_state.node_name.clone(),
                            disruption_reason: new_state.disruption_reason.clone(),
                            ..Default::default()
                        },
                    )
                    .await;

                // Mark the auto-stop as pending; the shared block below performs
                // (and retries) the scale-to-0 so a transient failure isn't lost.
                new_state.auto_stop_pending = true;
            }

            if max_crash_restart_count > 0
                && new_state.restart_count >= max_crash_restart_count
                && !is_intentional_stop
            {
                let prev_below_threshold =
                    prev_state.is_none_or(|p| p.restart_count < max_crash_restart_count);

                if prev_below_threshold {
                    warn!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        restart_count = new_state.restart_count,
                        threshold = max_crash_restart_count,
                        "Restart count exceeded threshold, auto-stopping sandbox"
                    );

                    // Fetch container logs for root cause before stopping
                    let logs = fetch_sandbox_logs(k8s, &pod_name, 50).await;

                    let mut error_parts = vec![format!(
                        "Sandbox auto-stopped after {} restarts (threshold: {}).",
                        new_state.restart_count, max_crash_restart_count
                    )];
                    if let Some(exit_code) = new_state.exit_code {
                        error_parts.push(format!("Exit code: `{exit_code}`"));
                    }
                    if let Some(ref detail) = new_state.failure_detail {
                        error_parts.push(format!("Failure: `{detail}`"));
                    }
                    if let Some(ref logs) = logs {
                        error_parts.push(format!("Container logs:\n```\n{logs}\n```"));
                    }
                    let detailed_error = error_parts.join("\n");

                    event_publisher
                        .publish_sandbox_event_with_payload(
                            &task_id,
                            SandboxStatus::Error,
                            &SandboxEventPayload {
                                message: detailed_error.clone(),
                                lifecycle_event: Some("auto_stopped".to_string()),
                                error: Some(detailed_error),
                                exit_code: new_state.effective_exit_code(),
                                terminated_reason: new_state.effective_terminated_reason(),
                                initiator: Some("crash_autostop".to_string()),
                                node_name: new_state.node_name.clone(),
                                disruption_reason: new_state.disruption_reason.clone(),
                                ..Default::default()
                            },
                        )
                        .await;

                    // Mark the auto-stop as pending; the shared block below does
                    // the scale-to-0 and retries it if it fails.
                    new_state.auto_stop_pending = true;
                }
            }

            // Single point that scales to 0 for a pending auto-stop (set by the
            // exit-78 fast-fail or the crash-restart threshold), clearing the
            // flag only on success. A failed scale leaves it set for the next
            // event or the periodic sweep to retry — without retry the
            // threshold never re-fires and the pod crash-loops forever.
            if new_state.auto_stop_pending
                && !is_intentional_stop
                && crash_autostop(deps, &task_id).await
            {
                info!(
                    task_id = %task_id,
                    pod_name = %pod_name,
                    "Crash auto-stop succeeded"
                );
                new_state.auto_stop_pending = false;
            }

            // `sandbox_completed` means the pod got a SIGTERM (the gRPC server
            // only exits via the SIGTERM graceful-shutdown driver), not that it
            // crashed. [`clean_exit_action`] reads intent, not the K8s API, to
            // tell a deliberate stop/remove apart from an involuntary node
            // event that still wants this pod rescheduled.
            if plan.has_completed_transition {
                match clean_exit_action(deps, k8s, &task_id).await {
                    CleanExitAction::AlreadyRemoved => {
                        info!(
                            task_id = %task_id,
                            pod_name = %pod_name,
                            "Sandbox exited cleanly after a remove; deployment is already deleted"
                        );
                    }
                    CleanExitAction::ConfirmStop => {
                        info!(
                            task_id = %task_id,
                            pod_name = %pod_name,
                            "Sandbox exited cleanly after an intentional stop, confirming scale to 0"
                        );
                        if let Err(e) = deployment::scale_deployment(k8s, &task_id, 0).await {
                            warn!(
                                task_id = %task_id,
                                error = %e,
                                "Failed to scale down completed sandbox"
                            );
                        }
                    }
                    CleanExitAction::LeaveForReschedule => {
                        info!(
                            task_id = %task_id,
                            pod_name = %pod_name,
                            "Sandbox exited cleanly while a replica is still desired; treating as involuntary eviction and leaving it to K8s to reschedule"
                        );
                    }
                }
            }

            // The task recovered, so a later failure is news again: no sibling
            // may still hand `replaces_published_failure` to the next pod.
            if became_ready {
                for sibling in pod_states
                    .values_mut()
                    .filter(|s| s.task_id == task_id && s.pod_name != pod_name)
                {
                    sibling.fatal_published = false;
                    sibling.replaces_published_failure = false;
                }
            }
            pod_states.insert(pod_name, new_state);
        }
        watcher::Event::Delete(pod) => {
            let task_id = if let Some(id) = extract_task_id(pod) {
                id
            } else {
                let name = pod.metadata.name.as_deref().unwrap_or("unknown");
                warn!(
                    pod_name = name,
                    "Deleted pod has no moatless.io/task-id label — skipping"
                );
                return;
            };
            let pod_name = pod.metadata.name.as_deref().unwrap_or("unknown");
            let operation_generation =
                pod.metadata.labels.as_ref().and_then(|labels| {
                    generation_from_label(labels.get(OPERATION_GENERATION_LABEL))
                });

            pod_states.remove(pod_name);

            let siblings = sibling_pods_for_task(pod_states, &task_id, pod_name);

            if siblings.is_empty() {
                // Distinguish Stopped vs Removed vs Recovering. A Deployment
                // that still desires replicas is replacing this pod after an
                // involuntary disruption, so publishing `stopped` here would
                // incorrectly make a recoverable eviction look intentional.
                let sandbox_status = match deployment::find_deployment(k8s, &task_id).await {
                    Ok(Some(dep)) => {
                        let desired = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
                        if desired == 0 {
                            info!(
                                task_id = %task_id,
                                pod_name = %pod_name,
                                "Pod deleted (Deployment scaled to 0) — publishing stopped"
                            );
                            Some(SandboxStatus::Stopped)
                        } else {
                            info!(
                                task_id = %task_id,
                                pod_name = %pod_name,
                                desired_replicas = desired,
                                "Pod deleted while Deployment still desires replicas — treating as recovering"
                            );
                            None
                        }
                    }
                    Ok(None) => {
                        // No Deployment → fully cleaned up
                        info!(
                            task_id = %task_id,
                            pod_name = %pod_name,
                            "Pod deleted (no Deployment) — publishing removed"
                        );
                        Some(SandboxStatus::Removed)
                    }
                    Err(e) => {
                        warn!(
                            task_id = %task_id,
                            error = %e,
                            "Failed to check Deployment on pod delete, defaulting to Removed"
                        );
                        Some(SandboxStatus::Removed)
                    }
                };

                let Some(sandbox_status) = sandbox_status else {
                    return;
                };

                let decision = classify_observation(
                    deps.lifecycle.as_ref(),
                    &task_id,
                    if sandbox_status == SandboxStatus::Stopped {
                        ObservedTransition::CleanStop
                    } else {
                        ObservedTransition::Deleted
                    },
                    operation_generation,
                )
                .await
                .publishes;
                if !decision {
                    debug!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        "Suppressing lifecycle-correlated pod deletion"
                    );
                }

                let delete_message = if sandbox_status == SandboxStatus::Stopped {
                    "Stopped"
                } else {
                    "Removed"
                };
                if decision {
                    event_publisher
                        .publish_sandbox_event_with_payload(
                            &task_id,
                            sandbox_status,
                            &SandboxEventPayload {
                                message: delete_message.to_string(),
                                lifecycle_event: Some("pod_deleted".to_string()),
                                ..Default::default()
                            },
                        )
                        .await;
                }

                // No sandbox.fatal here: "no Deployment + pod deleted" is
                // always intentional cleanup (it deletes the Deployment
                // first). Fatal events come only from the Apply handler, for
                // pod_failed/pod_evicted.
            } else {
                info!(
                    task_id = %task_id,
                    pod_name = %pod_name,
                    sibling_count = siblings.len(),
                    "Old pod deleted but siblings exist — skipping NotCreated"
                );
            }
        }
        watcher::Event::Init => {
            // Watcher re-syncing — clear stale entries so InitApply repopulates
            pod_states.clear();
        }
        watcher::Event::InitDone => {
            info!(
                pod_count = pod_states.len(),
                "Pod watcher initial sync complete"
            );
        }
    }
}
