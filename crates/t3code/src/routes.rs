use crate::assets;
use crate::attachments;
use crate::handlers::auth_session;
use crate::handlers::environment_descriptor;
use crate::handlers::scripts_list_http;
use crate::handlers::scripts_run_http;
use crate::handlers::shell_snapshot_http;
use crate::handlers::thread_snapshot_http;
use crate::handlers::websocket_upgrade;
use crate::oauth;
use crate::observability;
use crate::state::UiRpcState;
use axum::Router;
use axum::routing::get;
use axum::routing::post;
use tower_http::compression::CompressionLayer;

pub(crate) fn router(state: UiRpcState) -> Router {
    // The client pulls a thread body over HTTP rather than the socket so the
    // transport can compress it (T3's `threadSnapshotHttp.ts`), and unencoded a
    // long conversation is ~2MB against ~400KB gzipped. Scoped to the JSON reads
    // rather than applied globally because `/ws` shares this router.
    let compression = CompressionLayer::new();
    Router::new()
        .route("/.well-known/t3/environment", get(environment_descriptor))
        .route("/oauth/token", post(oauth::token_exchange))
        .route("/api/auth/websocket-ticket", post(oauth::websocket_ticket))
        .route("/api/auth/session", get(auth_session))
        .route(assets::ASSET_ROUTE_PATTERN, get(assets::serve))
        // The token in the path is the credential — see [`attachments`] — so
        // this one route is exempt from the auth middleware's cookie handling
        // rather than authenticated by it.
        .route(
            attachments::UPLOAD_ROUTE_PATTERN,
            post(attachments::receive),
        )
        .route(
            "/api/orchestration/shell",
            get(shell_snapshot_http).layer(compression.clone()),
        )
        .route(
            "/api/orchestration/threads/{thread_id}",
            get(thread_snapshot_http).layer(compression.clone()),
        )
        .route(
            "/api/orchestration/scripts/{thread_id}",
            get(scripts_list_http).layer(compression),
        )
        .route(
            "/api/orchestration/scripts/{thread_id}/{script_id}/run",
            post(scripts_run_http),
        )
        .route(observability::TRACES_ROUTE, post(observability::receive))
        .route("/ws", get(websocket_upgrade))
        .with_state(state)
}
