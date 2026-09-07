/// Where the Claude CLI writes its debug logs — pointed at `/tmp` to work around
/// an SDK lazy-init race. Child CLI processes inherit it from this process.
///
/// `docker/docker-entrypoint-sandbox.sh` exports the same value and remains the
/// source for image-entrypoint launches, but the K8s pod spec overrides the
/// entrypoint with `command: ["/usr/local/bin/sandbox"]` (see
/// `backend/src/sandbox/k8s/pod_builder.rs`), so on that path the script never
/// runs and the binary has to guarantee the variable itself.
const CLAUDE_CODE_DEBUG_LOGS_DIR: &str = "/tmp/claude-debug.log";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set before the Tokio runtime exists: `set_var` is only sound while the
    // process is single-threaded, and every consumer is a CLI child process
    // spawned much later, so the pre-runtime prologue is the one safe place.
    #[expect(
        unsafe_code,
        reason = "std::env::set_var is unsafe; sound here because the process is still single-threaded"
    )]
    // SAFETY: no runtime, no worker threads, and no other thread of any kind has
    // been created yet, so nothing can concurrently read the environment.
    unsafe {
        std::env::set_var("CLAUDE_CODE_DEBUG_LOGS_DIR", CLAUDE_CODE_DEBUG_LOGS_DIR);
    }

    moatless_sandbox::app::initialize_logging();
    // Rescue pods omit BACKEND_GRPC_URL; select this path before app::run
    // loads Config::from_env(), which requires the backend connection.
    let mode = std::env::var("SANDBOX_ONESHOT")
        .ok()
        .filter(|mode| !mode.is_empty());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        match mode.as_deref() {
            None => moatless_sandbox::app::run().await,
            Some("sync") => moatless_sandbox::workspace_sync::rescue_from_env().await,
            Some(other) => Err(format!("unknown SANDBOX_ONESHOT mode: {other}").into()),
        }
    });
    if let Err(error) = &result {
        if let Some(error) = error.downcast_ref::<moatless_sandbox::init::InitError>() {
            std::process::exit(match error {
                moatless_sandbox::init::InitError::Permanent(_) => 78,
                moatless_sandbox::init::InitError::Transient(_) => 1,
            });
        }
    }
    result
}
