//! The liveness and readiness probes, and the flag that gates them.

use super::fixtures::*;
use super::*;

#[test]
fn liveness_probe_absent_when_flag_disabled() {
    // Default config has `liveness_enabled: false` — no liveness probe is
    // attached, and the startup probe on /health is untouched.
    let config = test_config();
    let plan = test_plan();

    let container = build_sandbox_container(&config, "task-1", &plan, None, None).unwrap();
    assert!(
        container.liveness_probe.is_none(),
        "liveness probe must be absent when K8S_SANDBOX_LIVENESS_ENABLED is off"
    );
    assert!(
        container.startup_probe.is_some(),
        "startup probe must remain the startup gate regardless of the flag"
    );
}

#[test]
fn readiness_probe_tcp_checks_grpc_port() {
    // The backend reaches the sandbox only over gRPC, so pod readiness must
    // track the gRPC listener — not the HTTP `/health` startup gate. Without
    // this the pod stays in the Service endpoints while :50052 is unbound and
    // gRPC calls surface as connect timeouts / 503s.
    let config = test_config();
    let plan = test_plan();

    let container = build_sandbox_container(&config, "task-1", &plan, None, None).unwrap();
    let probe = container
        .readiness_probe
        .as_ref()
        .expect("readiness probe present on the sandbox container");
    let tcp = probe
        .tcp_socket
        .as_ref()
        .expect("readiness probe is a TCP-socket check");
    assert_eq!(
        tcp.port,
        IntOrString::Int(config.sandbox.grpc_port as i32),
        "readiness probe must target the gRPC control port, not the HTTP port"
    );
}

#[test]
fn liveness_probe_present_with_conservative_settings_when_flag_enabled() {
    let mut config = test_config();
    config.sandbox.liveness_enabled = true;
    let plan = test_plan();

    let container = build_sandbox_container(&config, "task-1", &plan, None, None).unwrap();
    let probe = container
        .liveness_probe
        .as_ref()
        .expect("liveness probe present when flag enabled");

    let http = probe.http_get.as_ref().expect("http_get action set");
    assert_eq!(http.path.as_deref(), Some("/livez"));
    assert_eq!(http.port, IntOrString::Int(8001));

    // ~5 minutes of failures before a restart; see `containers.rs`.
    assert_eq!(probe.period_seconds, Some(15));
    assert_eq!(probe.timeout_seconds, Some(10));
    assert_eq!(probe.failure_threshold, Some(20));

    // The startup probe still targets /health — liveness is additive.
    let startup = container.startup_probe.as_ref().expect("startup probe set");
    assert_eq!(
        startup.http_get.as_ref().and_then(|h| h.path.as_deref()),
        Some("/health")
    );
}

#[test]
fn liveness_probe_takes_its_timing_from_config() {
    let mut config = test_config();
    config.sandbox.liveness_enabled = true;
    config.sandbox.liveness_period_secs = 30;
    config.sandbox.liveness_timeout_secs = 20;
    config.sandbox.liveness_failure_threshold = 4;

    let container = build_sandbox_container(&config, "task-1", &test_plan(), None, None).unwrap();
    let probe = container
        .liveness_probe
        .as_ref()
        .expect("liveness probe present");

    assert_eq!(probe.period_seconds, Some(30));
    assert_eq!(probe.timeout_seconds, Some(20));
    assert_eq!(probe.failure_threshold, Some(4));
}

fn clone_stall_env(config: &Config) -> Option<String> {
    let plan = test_plan();
    let container = build_sandbox_container(config, "task-1", &plan, None, None).unwrap();
    container
        .env
        .unwrap_or_default()
        .into_iter()
        .find(|var| var.name == "GIT_CLONE_STALL_TIMEOUT_SECS")
        .and_then(|var| var.value)
}

#[test]
fn clone_stall_env_absent_when_unconfigured() {
    assert_eq!(clone_stall_env(&test_config()), None);
}

#[test]
fn clone_stall_env_forwards_a_value_inside_the_startup_window() {
    let mut config = test_config();
    config.sandbox.pod_startup_timeout_secs = 300;
    config.sandbox.git_clone_stall_timeout_secs = Some(180);

    assert_eq!(clone_stall_env(&config).as_deref(), Some("180"));
}

#[test]
fn clone_stall_env_is_capped_below_the_startup_window() {
    // 120s startup timeout gives a (120 / 5 + 6) * 5 = 150s probe window; a
    // stall limit past it would let the probe kill init before git gives up.
    let mut config = test_config();
    config.sandbox.pod_startup_timeout_secs = 120;
    config.sandbox.git_clone_stall_timeout_secs = Some(600);

    assert_eq!(clone_stall_env(&config).as_deref(), Some("120"));
}
