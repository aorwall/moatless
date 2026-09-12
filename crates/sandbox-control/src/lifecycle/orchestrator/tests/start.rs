//! Start: what a converged target suppresses, what each starting state
//! publishes, and the generation the runtime context carries.

use super::super::*;
use super::harness::*;
use moatless_sandbox_runtime::runtime::RuntimeError;

#[tokio::test]
async fn reusing_ready_sandbox_is_no_op_and_publishes_nothing() {
    let h = harness(open_running());
    h.runtime
        .ensure
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Ready)));
    let out = h.lifecycle.start("t").await.expect("ok");
    assert!(out.is_pure_no_op());
    assert!(h.events.facts().is_empty());
    assert!(h.store.marker_cleared());
}

#[tokio::test]
async fn start_noop_still_starting_does_not_sync_clear_marker() {
    // A no-op that observes Starting (reused a still-starting pod) is not
    // converged, so the marker is left for the watcher's Ready convergence.
    let h = harness(open_running());
    h.runtime
        .ensure
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Starting)));
    let out = h.lifecycle.start("t").await.expect("ok");
    assert!(out.is_pure_no_op());
    assert!(!h.store.marker_cleared());
    assert!(h.store.current().expect("intent").marker.is_some());
}

// ---- Start matrix (representative) ----

#[tokio::test]
async fn start_ready_synchronously_publishes_initializing_not_ready() {
    let h = harness(open_stopped());
    h.runtime
        .ensure
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Ready)));
    let out = h.lifecycle.start("t").await.expect("ok");
    assert_eq!(out.observed, ObservedStatus::Ready);
    assert!(h.events.facts().contains(&EventFact::Initializing));
    assert!(
        h.events
            .published
            .lock()
            .expect("lock")
            .iter()
            .all(|e| e.status != moatless_types::sandbox::SandboxStatus::Ready)
    );
}

#[tokio::test]
async fn runtime_success_diagnostics_are_republished_by_lifecycle() {
    use moatless_sandbox_runtime::outcome::{DegradationWarning, Diagnostics, ServerFailure};

    let h = harness(open_stopped());
    let diagnostics = Diagnostics {
        degradations: vec![DegradationWarning {
            code: "storage_degraded".into(),
            message: "using ephemeral storage".into(),
        }],
        server_failures: vec![ServerFailure {
            server: "preview".into(),
            message: "port was unavailable".into(),
        }],
    };
    h.runtime
        .ensure
        .lock()
        .expect("lock")
        .replace(Ok(
            RuntimeOutcome::applied(ObservedStatus::Starting).with_diagnostics(diagnostics.clone())
        ));

    let outcome = h.lifecycle.start("t").await.expect("ok");

    assert_eq!(outcome.diagnostics, diagnostics);
    assert_eq!(
        *h.events.diagnostics.lock().expect("lock"),
        vec![diagnostics]
    );
}

#[tokio::test]
async fn start_runtime_failure_publishes_one_error_and_keeps_intent() {
    let h = harness(open_stopped());
    h.runtime
        .ensure
        .lock()
        .expect("lock")
        .replace(Err(RuntimeError::Infrastructure(
            "previous pod still terminating".into(),
        )));
    let res = h.lifecycle.start("t").await;
    assert!(matches!(res, Err(LifecycleError::Runtime { .. })));
    assert_eq!(
        h.store.current().expect("intent").desired_state,
        SandboxDesiredState::Running
    );
    assert_eq!(h.events.facts(), vec![EventFact::Error]);

    // The adapter's diagnosis must reach the durable fact. Publishing a
    // bare Error here left the reason only in the caller's return value —
    // a log line, for a reconciler-driven start.
    let payload = h.events.published.lock().expect("lock")[0].payload();
    assert_eq!(
        payload.message,
        "Sandbox start failed: runtime infrastructure error: previous pod still terminating"
    );
    assert!(
        payload
            .error
            .as_deref()
            .is_some_and(|e| e.contains("previous pod still terminating"))
    );
    assert_eq!(
        payload.lifecycle_event.as_deref(),
        Some("runtime_infrastructure")
    );
}

// ---- Correlation: generation reaches the adapter ----

#[tokio::test]
async fn generation_reaches_the_runtime_context() {
    let h = harness(open_stopped());
    h.runtime
        .ensure
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Starting)));
    h.lifecycle.start("t").await.expect("ok");
    let ctx = h.runtime.last_ctx.lock().expect("lock").expect("ctx");
    assert_eq!(ctx.operation_generation, OperationGeneration(1));
}
