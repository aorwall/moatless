#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Root-supervision behaviour: what the backend process does when one of its
//! long-lived tasks terminates, and how it shuts them all down.
//!
//! These exercise the seam `main` composes — `Supervisor` plus the `/ready`
//! probe it feeds — rather than any individual worker, so each of the ticket's
//! verification cases is proved once at the level it actually matters.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once, PoisonError};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;

use moatless_backend::startup::readiness_router;
use moatless_backend::supervisor::{Criticality, ExitCause, Supervisor};

// =========================================================================
// Harness
// =========================================================================

/// Captures the process-wide tracing output so tests can assert on what the
/// supervisor *reported*, not just what it returned.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    fn contents(&self) -> String {
        let buffer = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&buffer).into_owned()
    }
}

impl Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

static LOGS_INIT: Once = Once::new();
static LOGS: Mutex<Option<CapturedLogs>> = Mutex::new(None);

/// Install the capturing subscriber once for the whole test binary. Tests run
/// in parallel against one buffer, so each asserts on task names unique to it.
fn captured_logs() -> CapturedLogs {
    LOGS_INIT.call_once(|| {
        let logs = CapturedLogs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_max_level(tracing::Level::INFO)
            .with_ansi(false)
            .finish();
        // Only this binary sets a global subscriber, so it cannot already be set.
        tracing::subscriber::set_global_default(subscriber).expect("install test subscriber");
        *LOGS.lock().unwrap_or_else(PoisonError::into_inner) = Some(logs);
    });
    LOGS.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .expect("captured logs installed")
}

/// A well-behaved worker: runs until root cancellation, then records that it
/// saw it and returns.
fn cooperative(cancel: CancellationToken, saw_cancel: Arc<AtomicBool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        cancel.cancelled().await;
        saw_cancel.store(true, Ordering::SeqCst);
    })
}

/// A worker that never observes cancellation — the straggler case.
fn uncancellable() -> JoinHandle<()> {
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_mins(1)).await;
    })
}

async fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "condition never became true");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn ready_response(
    health: Arc<moatless_backend::supervisor::SupervisorHealth>,
) -> (StatusCode, serde_json::Value) {
    ready_response_with_database(None, health).await
}

async fn ready_response_with_database(
    database: Option<sqlx::PgPool>,
    health: Arc<moatless_backend::supervisor::SupervisorHealth>,
) -> (StatusCode, serde_json::Value) {
    let response = readiness_router(database, health)
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("readiness probe responded");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    (status, serde_json::from_slice(&body).expect("json body"))
}

// =========================================================================
// Critical failure: panic and early return
// =========================================================================

#[tokio::test]
async fn critical_panic_terminates_the_process_with_a_structured_failure() {
    let cancel = CancellationToken::new();
    let mut supervisor = Supervisor::new(cancel.clone());
    supervisor.supervise(
        "panicking-critical",
        Criticality::Fatal,
        tokio::spawn(async { panic!("synthetic critical panic") }),
    );

    let exit = supervisor
        .run(Duration::from_secs(5))
        .await
        .expect_err("a fatal panic must end the process");

    assert_eq!(exit.name, "panicking-critical");
    match &exit.cause {
        ExitCause::Panic(message) => assert!(
            message.contains("synthetic critical panic"),
            "panic message lost: {message}"
        ),
        other => panic!("expected a panic cause, got {other}"),
    }
    // The failure reaches `main` as an error, which is what exits non-zero.
    let as_error: &dyn std::error::Error = &exit;
    assert!(as_error.to_string().contains("panicking-critical"));
}

#[tokio::test]
async fn critical_early_return_terminates_the_process() {
    let cancel = CancellationToken::new();
    let mut supervisor = Supervisor::new(cancel.clone());
    // A server whose `serve` loop ends on its own — no panic, no error value.
    supervisor.supervise(
        "returning-server",
        Criticality::Fatal,
        tokio::spawn(async {}),
    );

    let exit = supervisor
        .run(Duration::from_secs(5))
        .await
        .expect_err("an early server return must end the process");

    assert_eq!(exit.name, "returning-server");
    assert!(matches!(exit.cause, ExitCause::EarlyReturn));
}

#[tokio::test]
async fn a_fatal_exit_cancels_every_other_owned_task() {
    let cancel = CancellationToken::new();
    let mut supervisor = Supervisor::new(cancel.clone());
    let saw_cancel = Arc::new(AtomicBool::new(false));
    supervisor.supervise(
        "cooperative-worker",
        Criticality::Readiness,
        cooperative(cancel.clone(), Arc::clone(&saw_cancel)),
    );
    supervisor.supervise(
        "failing-critical",
        Criticality::Fatal,
        tokio::spawn(async {}),
    );

    supervisor
        .run(Duration::from_secs(5))
        .await
        .expect_err("fatal exit");

    assert!(cancel.is_cancelled(), "root token must be cancelled");
    assert!(
        saw_cancel.load(Ordering::SeqCst),
        "the surviving task must be cancelled and joined"
    );
}

// =========================================================================
// Optional worker exit policy
// =========================================================================

#[tokio::test]
async fn an_optional_worker_exit_is_tolerated_and_leaves_readiness_intact() {
    let cancel = CancellationToken::new();
    let mut supervisor = Supervisor::new(cancel.clone());
    let health = supervisor.health();
    let saw_cancel = Arc::new(AtomicBool::new(false));

    supervisor.supervise(
        "optional-poller",
        Criticality::Optional,
        tokio::spawn(async {}),
    );
    supervisor.supervise(
        "surviving-critical",
        Criticality::Fatal,
        cooperative(cancel.clone(), Arc::clone(&saw_cancel)),
    );

    let running = tokio::spawn(supervisor.run(Duration::from_secs(5)));
    // Give the optional exit time to be observed and acted on.
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert!(!running.is_finished(), "the process must keep running");
    assert!(
        health.failed_tasks().is_empty(),
        "an optional worker must not affect readiness"
    );

    cancel.cancel();
    running
        .await
        .expect("supervisor joined")
        .expect("an optional exit is not a process failure");
    let (status, _) = ready_response(health).await;
    assert_eq!(status, StatusCode::OK);
}

// =========================================================================
// Readiness policy
// =========================================================================

#[tokio::test]
async fn a_required_worker_exit_flips_readiness_without_killing_the_process() {
    let cancel = CancellationToken::new();
    let mut supervisor = Supervisor::new(cancel.clone());
    let health = supervisor.health();

    let (status, body) = ready_response(Arc::clone(&health)).await;
    assert_eq!(status, StatusCode::OK, "healthy before anything fails");
    assert_eq!(body["status"], "ok");

    supervisor.supervise(
        "required-consumer",
        Criticality::Readiness,
        tokio::spawn(async {}),
    );
    supervisor.supervise(
        "readiness-critical",
        Criticality::Fatal,
        cooperative(cancel.clone(), Arc::new(AtomicBool::new(false))),
    );

    let running = tokio::spawn(supervisor.run(Duration::from_secs(5)));
    wait_until(|| !health.failed_tasks().is_empty()).await;

    assert_eq!(health.failed_tasks(), vec!["required-consumer"]);
    assert!(
        !running.is_finished(),
        "a required worker's exit must not end the process"
    );

    let (status, body) = ready_response(Arc::clone(&health)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "unavailable");
    assert_eq!(body["component"], "supervisor");
    assert_eq!(body["tasks"], serde_json::json!(["required-consumer"]));

    cancel.cancel();
    running
        .await
        .expect("supervisor joined")
        .expect("not fatal");
}

#[tokio::test]
async fn a_database_pool_that_cannot_serve_queries_flips_readiness() {
    let cancel = CancellationToken::new();
    let supervisor = Supervisor::new(cancel);
    let health = supervisor.health();
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://readiness:readiness@127.0.0.1:1/readiness")
        .expect("synthetic pool");
    pool.close().await;

    let (status, body) = ready_response_with_database(Some(pool), health).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "unavailable");
    assert_eq!(body["component"], "database");
}

// =========================================================================
// Cancellation and bounded shutdown
// =========================================================================

#[tokio::test]
async fn root_cancellation_reaches_every_owned_task() {
    let cancel = CancellationToken::new();
    let mut supervisor = Supervisor::new(cancel.clone());
    let flags: Vec<Arc<AtomicBool>> = (0..4).map(|_| Arc::new(AtomicBool::new(false))).collect();
    let names = [
        "cancel-a",
        "cancel-b",
        "cancel-c-optional",
        "cancel-d-critical",
    ];
    let criticalities = [
        Criticality::Readiness,
        Criticality::Readiness,
        Criticality::Optional,
        Criticality::Fatal,
    ];
    for ((name, criticality), flag) in names.iter().zip(criticalities).zip(&flags) {
        supervisor.supervise(
            name,
            criticality,
            cooperative(cancel.clone(), Arc::clone(flag)),
        );
    }

    let running = tokio::spawn(supervisor.run(Duration::from_secs(5)));
    cancel.cancel();
    running
        .await
        .expect("supervisor joined")
        .expect("shutdown is not a failure");

    for (name, flag) in names.iter().zip(&flags) {
        assert!(
            flag.load(Ordering::SeqCst),
            "cancellation never reached {name}"
        );
    }
}

#[tokio::test]
async fn shutdown_joins_within_the_deadline_and_reports_stragglers() {
    let logs = captured_logs();
    let cancel = CancellationToken::new();
    let mut supervisor = Supervisor::new(cancel.clone());
    let saw_cancel = Arc::new(AtomicBool::new(false));
    supervisor.supervise(
        "straggler-ignoring-cancellation",
        Criticality::Readiness,
        uncancellable(),
    );
    supervisor.supervise(
        "prompt-worker",
        Criticality::Readiness,
        cooperative(cancel.clone(), Arc::clone(&saw_cancel)),
    );

    cancel.cancel();
    let started = Instant::now();
    supervisor
        .run(Duration::from_millis(200))
        .await
        .expect("a straggler is not a fatal exit");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(30),
        "shutdown waited on the straggler's full one-minute sleep ({elapsed:?})"
    );
    assert!(
        saw_cancel.load(Ordering::SeqCst),
        "the prompt worker should still have been joined"
    );
    let output = logs.contents();
    assert!(
        output.contains("Shutdown deadline exceeded"),
        "no straggler report in logs: {output}"
    );
    assert!(
        output.contains("straggler-ignoring-cancellation"),
        "straggler not named in the report: {output}"
    );
}
