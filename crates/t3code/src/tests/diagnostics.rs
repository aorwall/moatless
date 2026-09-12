//! Reading the fleet: the mapped result, and that a slow read holds nothing
//! behind it.

use super::fixtures::*;
use super::*;

/// A fleet with one Sandbox in it, for the dispatch test below. Real
/// mapping from pod list and metrics into this shape is unit-tested
/// against fixtures in `sandbox::k8s::pod_reader`; this only checks that
/// the RPC arm serializes what the seam hands it, unchanged.
struct FakeFleet;

#[async_trait]
impl ClusterDiagnostics for FakeFleet {
    async fn list_sandbox_diagnostics(&self) -> Result<Vec<SandboxDiagnosticsEntry>, SandboxError> {
        Ok(vec![SandboxDiagnosticsEntry {
            task_id: "task-1".into(),
            title: None,
            sandbox_status: SandboxStatus::Ready,
            node_name: Some("node-a".into()),
            containers: vec![SandboxDiagnosticsContainer {
                name: "sandbox".into(),
                state: "running".into(),
                restart_count: 0,
                cpu_millicores: Some(100),
                memory_mb: Some(256),
            }],
            total_cpu_millicores: Some(100),
            total_memory_mb: Some(256),
        }])
    }
}

#[tokio::test]
async fn a_sandbox_list_diagnostics_request_returns_the_mapped_fleet() {
    let mut state = state();
    state.sandbox.diagnostics = Arc::new(FakeFleet);
    let frame = request("1", "sandbox.listDiagnostics", json!({}));
    let mut connection = connection();
    let output = connection.exchange(&frame, &state, &viewer()).await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    let sandboxes = &output.frames[0]["exit"]["value"]["sandboxes"];
    assert_eq!(sandboxes[0]["taskId"], "task-1");
    assert_eq!(sandboxes[0]["nodeName"], "node-a");
    assert_eq!(sandboxes[0]["containers"][0]["name"], "sandbox");
    assert_eq!(sandboxes[0]["containers"][0]["cpuMillicores"], 100);
    assert_eq!(sandboxes[0]["totalMemoryMb"], 256);
}

/// A fleet read that answers only once the test releases it, for the ordering
/// test below.
struct BlockedFleet(Arc<Notify>);

#[async_trait]
impl ClusterDiagnostics for BlockedFleet {
    async fn list_sandbox_diagnostics(&self) -> Result<Vec<SandboxDiagnosticsEntry>, SandboxError> {
        self.0.notified().await;
        Ok(Vec::new())
    }
}

/// The whole point of detaching: a slow read holds only itself. Before it, one
/// connection answered one request at a time, so everything a client had sent
/// sat unread on the socket behind the slowest handler in the batch.
// Paused time, so the deadline below costs nothing to reach: an inline read
// would sit on `BlockedFleet` forever, and a hung test says less than a failed
// one.
#[tokio::test(start_paused = true)]
async fn a_slow_read_does_not_hold_the_next_request_behind_it() {
    let released = Arc::new(Notify::new());
    let mut state = state();
    state.sandbox.diagnostics = Arc::new(BlockedFleet(released.clone()));
    let mut connection = connection();

    let slow = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        connection.session.handle_json(
            &request("1", "sandbox.listDiagnostics", json!({})),
            &state,
            &viewer(),
        ),
    )
    .await
    .expect("the frame loop is not waiting on the read");
    assert!(
        slow.frames.is_empty(),
        "the read has not answered, and the loop did not wait for it"
    );

    let fast = connection
        .session
        .handle_json(&request("2", "server.probe", json!({})), &state, &viewer())
        .await;
    assert_eq!(
        fast.frames[0]["requestId"], "2",
        "answered while the read is still running"
    );

    released.notify_one();
    connection.session.detached.wait_until_idle().await;
    let answered = connection
        .next_pushed()
        .expect("the read answers in the end");
    assert_eq!(answered["requestId"], "1");
    assert_eq!(answered["exit"]["_tag"], "Success");
}
