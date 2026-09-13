/// Tracks the current initialization phase.
///
/// Stored in `SandboxState` as `Arc<RwLock<InitPhase>>`; `/health`, gRPC
/// `CheckHealth`, and the message consumer all treat it as
/// unhealthy/not-yet-ready until it reaches `Ready`.
#[derive(Debug, Clone)]
pub enum InitPhase {
    Starting,
    FetchingCredentials,
    CloningRepository,
    Ready,
    Failed(String),
}
