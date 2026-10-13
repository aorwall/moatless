use super::Ports;
use crate::{
    access_settings, adapters, feature_flags, file_storage, repository_sync, sandbox,
    state::AppState, t3code,
};
use axum::{Json, Router};
use moatless_core::{config::Config, feature_flags::FeatureFlags};
use moatless_identity::{auth, secret};
use moatless_loops::loops;
use moatless_tasks::{agent, artifact, feedback, message, tag, task, task_event};
use moatless_workspaces::{plugin, repository, skill, workspace};
use std::sync::Arc;
use tracing::info;

pub(crate) fn domain_routes(app_state: &AppState) -> (Router, utoipa::openapi::OpenApi) {
    let modules = vec![
        agent::routes::agent_router(app_state.clone()),
        task::routes::task_router(app_state.clone()),
        feedback::routes::feedback_router(app_state.clone()),
        artifact::routes::artifact_router(app_state.clone()),
        message::routes::message_router(app_state.clone()),
        file_storage::routes::file_storage_router(app_state.clone()),
        repository::routes::repository_router(app_state.clone()),
        repository_sync::routes::repository_sync_router(app_state.clone()),
        secret::routes::secret_router(app_state.clone()),
        tag::routes::tag_router(app_state.clone()),
        access_settings::routes::access_settings_router(app_state.clone()),
        adapters::routes::adapter_router(app_state.clone()),
        loops::http::loops_router(app_state.clone()),
        moatless_identity::compose::team_router(
            app_state.db.clone(),
            app_state.token_verifier.clone(),
        ),
        moatless_decisions::compose::decisions_router(
            &app_state.config.decision,
            app_state.config.http.decision,
            app_state.token_verifier.clone(),
        ),
        task_event::routes::task_event_router(app_state.clone()),
        plugin::routes::plugin_router(app_state.clone()),
        skill::routes::skill_router(app_state.clone()),
        workspace::routes::workspace_router(app_state.clone()),
    ];
    let mut router = Router::new();
    let mut api = utoipa::openapi::OpenApi::default();
    for (mod_router, mod_api) in modules {
        router = router.merge(mod_router);
        api.merge(mod_api);
    }
    // Historical task-events replay. Authorizes the caller against the task
    // (AuthUser), so it lives on the AppState router rather than the SSE
    // firehose router.
    router = router.merge(crate::sse::routes::task_events_router(app_state.clone()));
    (router, api)
}

pub(crate) struct UiRpcInputs {
    pub connector: Arc<dyn sandbox_proto::client::SandboxConnector>,
    pub snapshots: Option<Arc<dyn moatless_ports::sandbox::WorkspaceSnapshotSource>>,
    pub observer: Arc<dyn sandbox::status_reader::SandboxStatusReader>,
    pub diagnostics: Arc<dyn moatless_ports::sandbox::ClusterDiagnostics>,
    pub ports: Arc<dyn moatless_ports::sandbox::SandboxPortPublisher>,
}

pub(crate) async fn ui_rpc(
    config: &Config,
    ports: &Ports,
    app_state: &AppState,
    inputs: UiRpcInputs,
) -> Result<Option<moatless_t3code::UiRpc>, Box<dyn std::error::Error>> {
    moatless_t3code::router_if_enabled(
        &ports.t3_graph,
        moatless_t3code::UiRpcDependencies {
            session_cookie_name: config.auth.session_cookie_name.clone(),
            session_jwt_secret: auth::jwt::get_jwt_secret(config).map(String::from),
            pairing: ports.pairing.clone(),
            workspace_script_commands: app_state.workspaces.clone(),
            workspace_scripts: Arc::new(moatless_t3code::owners::TaskWorkspaceScripts {
                tasks: app_state.task_reader.clone(),
                workspaces: app_state.workspaces.clone(),
            }),
            sandboxes: inputs.connector,
            snapshots: inputs.snapshots,
            observer: moatless_sandbox_control::observation::contract_observer(
                inputs.observer,
                Some(ports.turn_counter.clone()),
            ),
            diagnostics: inputs.diagnostics,
            ports: inputs.ports,
            intent: app_state.sandbox.clone(),
            starter: app_state.sandbox.clone(),
            stopper: app_state.sandbox.clone(),
            operator: app_state.sandbox.clone(),
            preview_tab_ttl: moatless_t3code::preview_tabs::PreviewTabTtl(
                std::time::Duration::from_secs(config.sandbox.idle_timeout_minutes * 60),
            ),
            agents: Arc::new(moatless_t3code::owners::AgentCatalog {
                secrets: app_state.secrets.clone(),
                harness_credentials: app_state.harness_credentials.clone(),
            }),
            branches: app_state.repository_branches.clone(),
            skills: Arc::new(moatless_t3code::owners::WorkspaceSkills {
                workspaces: app_state.workspaces.clone(),
                skills: app_state.skills.clone(),
            }),
            relations: app_state.task_relations.clone(),
            pull_requests: Arc::new(t3code::backend::AppStateBoundPullRequests(
                app_state.clone(),
                ports.t3_graph.reads().clone(),
            )),
            source_control_auth: Arc::new(moatless_t3code::owners::GithubAuth {
                credentials: app_state.credentials.clone(),
                git_host_tokens: app_state.git_host_tokens.clone(),
            }),
            commands: Arc::new(t3code::backend::AppStateCommands(app_state.clone())),
            attachments: Arc::new(t3code::backend::AppStateAttachments(app_state.clone())),
            model_defaults: moatless_t3code::AgentModelDefaults {
                claude_code: config.sandbox.default_model.clone(),
                codex: Some(config.sandbox.default_codex_model.clone()),
            },
            otlp_traces_enabled: config.otel.otlp_endpoint.is_some(),
            frontend_url: config.auth.frontend_url.clone(),
            client_traces_http: config.http.client_traces,
            usage_limits_http: config.http.usage_limits,
        },
        config.t3_ui_rpc.clone(),
    )
    .await
    .map_err(Into::into)
}

pub(crate) fn finish(
    mut router: Router,
    mut api: utoipa::openapi::OpenApi,
    feature_flags: FeatureFlags,
    device_pairing: bool,
    sse_state: crate::sse::routes::SseState,
    auth_middleware_state: Option<auth::middleware::AuthMiddlewareState>,
) -> Router {
    // Routes that don't require a database
    let (feature_flags_router, feature_flags_api) =
        feature_flags::feature_flags_router(feature_flags, device_pairing);
    router = router.merge(feature_flags_router);
    api.merge(feature_flags_api);
    router = router.merge(crate::sse::routes::sse_router(sse_state));
    info!("SSE routes registered");

    let openapi_json = api.clone();
    let router = router.route(
        "/openapi.json",
        axum::routing::get(move || async move { Json(openapi_json) }),
    );

    // Apply auth middleware. Runs inside the request-trace span (added below)
    // so it can record `user_id` onto the current span.
    let router = if let Some(mw_state) = auth_middleware_state {
        info!("Auth middleware enabled");
        router
            .layer(axum::middleware::from_fn(auth::middleware::auth_middleware))
            .layer(axum::Extension(mw_state))
    } else {
        info!("Auth middleware disabled (no database)");
        router
    };

    // Outermost layer — opens one `http_request` span (request_id, method,
    // path, …) per request so every downstream log line inherits the context,
    // emits a single access-log line, and echoes `x-request-id` to clients.
    router.layer(axum::middleware::from_fn(
        crate::http_trace::request_trace_middleware,
    ))
}
