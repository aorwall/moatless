//! Sandbox initialization orchestrator.
//!
//! Handles the full lifecycle of workspace setup in two phases:
//!
//! **Phase 1 (core init)**: Credential fetching, git authentication,
//! repository cloning, plugin sync. Health endpoint returns 503 during
//! this phase. On completion, `InitPhase::Ready` is set and health returns 200.
//!
//! **Phase 2 (workspace setup)**: Runs after health is ready. Copies
//! prebuilt deps, sets up build cache, runs setup commands.
//! These are non-fatal — failures are logged but don't affect health.

pub(crate) mod credentials;
mod dirs;
mod git;
mod workspace;

use moatless_events::SandboxSubType;
use sandbox_proto::PublishEventRequest;
use tracing::{error, info, warn};

use crate::state::SandboxState;

/// Tracks the current initialization phase.
///
/// Stored in `SandboxState` as `Arc<RwLock<InitPhase>>` and read by:
/// - HTTP `/health` endpoint (returns 503 until Ready)
/// - gRPC `CheckHealth` (returns healthy=true only when Ready)
/// - Message consumer (waits for Ready before polling)
#[derive(Debug, Clone)]
pub enum InitPhase {
    Starting,
    FetchingCredentials,
    CloningRepository,
    Ready,
    Failed(String),
}

/// Publish a sandbox init event via gRPC.
///
/// These events appear in the frontend's StartupTimeline alongside K8s events.
/// Uses `EventType::Sandbox` with `SandboxSubType::Initializing` — the sandbox
/// IS initializing while these events fire. Detailed info goes in the payload.
async fn publish_sandbox_event(
    state: &SandboxState,
    reason: &str,
    message: &str,
    is_warning: bool,
) {
    let payload = serde_json::json!({
        "reason": reason,
        "message": message,
        "eventType": if is_warning { "Warning" } else { "Normal" },
    });

    let payload_json = match serde_json::to_string(&payload) {
        Ok(json) => json,
        Err(e) => {
            warn!(error = %e, "Failed to serialize sandbox event payload");
            return;
        }
    };

    let proto_event_type: sandbox_proto::EventType = moatless_events::EventType::Sandbox.into();

    let req = PublishEventRequest {
        task_id: state.config.task_id.clone(),
        event_type: proto_event_type as i32,
        sub_type: SandboxSubType::Initializing.as_str().to_string(),
        payload_json,
    };

    state.events.client().publish_event(req).await;
}

/// Run the full initialization sequence (phase 1 + phase 2).
///
/// Called from `main.rs` via `tokio::spawn`. Phase 1 runs first and sets
/// `InitPhase::Ready` on success (or `Failed` on error). Phase 2 runs
/// immediately after Ready, performing workspace setup that doesn't need
/// to block the health endpoint.
pub async fn run_init(state: SandboxState) {
    info!(task_id = %state.config.task_id, "Starting sandbox initialization");

    // Phase 1: Core init — must complete before health returns 200
    if let Err(e) = run_core_init(&state).await {
        error!(error = %e, "Core initialization failed");
        // Write termination message for K8s to surface in pod status.
        // Then exit — K8s will restart the container and eventually trigger
        // CrashLoopBackOff, which the readiness checker detects as a permanent failure.
        let _ = std::fs::write("/dev/termination-log", &e);
        std::process::exit(1);
    }

    // Mark as ready — health endpoint now returns 200
    *state.init_phase.write().await = InitPhase::Ready;

    publish_sandbox_event(
        &state,
        "SandboxReady",
        "Core initialization complete, starting workspace setup",
        false,
    )
    .await;

    info!(task_id = %state.config.task_id, "Core init complete — health is now ready");

    // Phase 2: Workspace setup — runs after health is 200
    run_workspace_setup(&state).await;

    // Write workspace-ready sentinel AFTER setup commands complete (e.g. bun install).
    // Server containers poll for this file before starting, so they must wait
    // until dependencies are installed.
    let ready_path = format!("{}/.workspace-ready", state.config.data_dir);
    if let Err(e) = tokio::fs::write(&ready_path, "ready").await {
        warn!(error = %e, "Failed to write workspace-ready sentinel");
    }

    info!(task_id = %state.config.task_id, "Workspace setup complete");
}

/// Phase 1: Core initialization.
///
/// Performs essential steps that must complete before the pod is marked healthy:
/// 1. Ensure directories and symlinks
/// 2. Fetch and apply credentials (fatal on failure)
/// 3. Configure git authentication (non-fatal)
/// 4. Clone repository (fatal on failure)
/// 5. Sync plugins (non-fatal)
async fn run_core_init(state: &SandboxState) -> Result<(), String> {
    // 1. Ensure directories exist
    dirs::ensure_directories(&state.config);

    // 2. Fetch credentials from backend via gRPC
    {
        *state.init_phase.write().await = InitPhase::FetchingCredentials;
    }

    if let Err(e) = credentials::fetch_and_apply_credentials(state).await {
        let msg = format!("Failed to fetch credentials: {e}");
        publish_sandbox_event(state, "CredentialsFailed", &msg, true).await;
        return Err(msg);
    }
    publish_sandbox_event(
        state,
        "CredentialsFetched",
        "Fetched credentials from backend",
        false,
    )
    .await;

    // 3. Configure git authentication (non-fatal)
    if let Err(e) = git::configure_git(&state.config).await {
        warn!(error = %e, "Git configuration failed (non-fatal)");
    } else {
        publish_sandbox_event(
            state,
            "GitConfigured",
            "Configured git authentication",
            false,
        )
        .await;
    }

    // 4. Clone repository (fatal on failure)
    {
        *state.init_phase.write().await = InitPhase::CloningRepository;
    }

    if let Some(ref clone_url) = state.config.git_clone_url {
        if !clone_url.is_empty() {
            match git::clone_repository(&state.config).await {
                Ok(()) => {
                    let branch_info = state
                        .config
                        .git_branch
                        .as_deref()
                        .map(|b| format!(" ({b})"))
                        .unwrap_or_default();
                    publish_sandbox_event(
                        state,
                        "RepositoryCloned",
                        &format!("Cloned repository{branch_info}"),
                        false,
                    )
                    .await;
                }
                Err(e) => {
                    let msg = format!("Failed to clone repository: {e}");
                    publish_sandbox_event(state, "RepositoryCloneFailed", &msg, true).await;
                    return Err(msg);
                }
            }
        }
    }

    // 5. Sync plugins (non-fatal)
    if !state.config.plugin_ids.is_empty() {
        match workspace::sync_plugins(&state.config).await {
            Ok(count) => {
                publish_sandbox_event(
                    state,
                    "PluginsSynced",
                    &format!("Synced {count} plugin(s)"),
                    false,
                )
                .await;
            }
            Err(e) => {
                warn!(error = %e, "Plugin sync failed (non-fatal)");
            }
        }
    }

    Ok(())
}

/// Phase 2: Workspace setup.
///
/// Runs after health is ready. All steps are non-fatal — failures are logged
/// and published as warning events but don't affect pod health.
///
/// Steps:
/// 1. Copy prebuilt dependencies (node_modules, etc.)
/// 2. Set up build cache
/// 3. Run setup commands
async fn run_workspace_setup(state: &SandboxState) {
    workspace::copy_prebuilt_deps(&state.config).await;
    workspace::setup_build_cache(&state.config).await;

    for cmd in &state.config.setup_commands {
        publish_sandbox_event(
            state,
            "SetupRunning",
            &format!("Running setup command: {cmd}"),
            false,
        )
        .await;
        if let Err(e) = workspace::run_setup_command(&state.config, cmd).await {
            let msg = format!("Setup command failed: {cmd}: {e}");
            warn!("{msg}");
            publish_sandbox_event(state, "SetupFailed", &msg, true).await;
            // Non-fatal — continue with remaining setup
        }
    }

    publish_sandbox_event(
        state,
        "WorkspaceSetupComplete",
        "Workspace setup complete",
        false,
    )
    .await;
}
