/// Where the Claude CLI writes its debug logs, under the temp dir, to work
/// around an SDK lazy-init race; child CLI processes inherit it.
///
/// K8s overrides the entrypoint script that also sets this (see
/// `backend/src/sandbox/k8s/pod_builder.rs`), so the binary sets it itself.
const CLAUDE_CODE_DEBUG_LOGS_FILE: &str = "claude-debug.log";

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
        std::env::set_var(
            "CLAUDE_CODE_DEBUG_LOGS_DIR",
            std::env::temp_dir().join(CLAUDE_CODE_DEBUG_LOGS_FILE),
        );
        // Exported here rather than onto one child's env map so that every
        // spawn inherits the cap — the agent CLI, a `moat cmd` build, a
        // terminal — since any of them can be the process that hits it.
        for (key, value) in
            moatless_sandbox::limits::memory_env_exports(std::env::var_os("NODE_OPTIONS").is_some())
        {
            std::env::set_var(key, value);
        }
    }

    // Read here rather than through `Config::from_env()`: that call panics if
    // `BACKEND_GRPC_URL` is unset, and a one-shot mode carries no such thing.
    let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    moatless_sandbox::app::initialize_logging(&log_level);
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
            Some("sync") => {
                let config = moatless_sandbox::config::RescueConfig::from_env()?;
                moatless_sandbox::workspace_sync::rescue(config).await
            }
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
