use crate::init::InitPhase;
use crate::state::SandboxState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

/// GET /health — returns 200 when init is Ready, 503 otherwise.
///
/// K8s startup probe will retry until Ready, at which point the pod
/// becomes eligible for traffic.
pub(super) async fn health(State(state): State<SandboxState>) -> impl IntoResponse {
    let phase = state.init_phase.read().await;
    match *phase {
        InitPhase::Ready => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// GET /livez — honest liveness signal for the pod's own machinery.
///
/// Returns 200 while the pod can still make progress and 503 once it cannot, so
/// K8s can restart a hung-but-running pod that `/health` (a startup probe) would
/// keep reporting healthy. Two ways to lose it: a supervised task terminated
/// early, or the agent lifecycle task wedged or exited. Still depends on neither backend
/// reachability, credentials, nor disk, so a transient dependency outage can't
/// trigger a restart that kills an in-flight turn.
///
/// The decision itself is [`SandboxState::is_live`] — in the library, where it
/// is testable.
pub(super) async fn livez(State(state): State<SandboxState>) -> impl IntoResponse {
    if state.is_live().await {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
