use crate::init::InitPhase;
use crate::state::SandboxState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

/// GET /health — returns 200 when init is Ready, 503 otherwise.
///
/// K8s startup probe retries until Ready, then the pod becomes eligible for
/// traffic.
pub(super) async fn health(State(state): State<SandboxState>) -> impl IntoResponse {
    let phase = state.init_phase.read().await;
    match *phase {
        InitPhase::Ready => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// GET /livez — honest liveness signal for the pod's own machinery.
///
/// See [`SandboxState::is_live`] for what fails it, and why.
pub(super) async fn livez(State(state): State<SandboxState>) -> impl IntoResponse {
    if state.is_live().await {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
