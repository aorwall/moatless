//! Artifact HTTP handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::Multipart;
use axum_extra::extract::multipart::MultipartError;

use crate::credential_sync::CredentialSync;
use crate::message::send::MessageSendService;
use crate::task::creation::TaskCreationService;
use crate::task::fork::{ForkCreationPorts, TaskForkService};
use crate::task::operations::TaskCommands;
use crate::task::presenter::TaskPresenter;
use crate::task::service::CreateTaskPorts;
use crate::task::switch::TaskSwitchService;
use moatless_core::config::Config;
use moatless_core::http_auth::AuthUser;
use moatless_ports::identity::{SecretCatalog, UserResolver};
use moatless_types::task::Visibility;
use moatless_workspace_ports::{RepositoryReader, WorkspaceResolver};

use super::error::ArtifactError;
use super::models::{InlineRequest, PublishRequest};
use super::operations::{Artifacts, PAGE_CONTENT_TYPE};
use super::rules::RecordRules;
use super::schemas::{
    ArtifactResponse, ForkArtifactRequest, ForkArtifactResponse, ListArtifactsResponse,
    PublishArtifactBody, SetArtifactVisibilityRequest,
};
use super::service::MAX_PAGE_BYTES;

/// Replaces axum's 2 MB default on the publish route: a full page plus room
/// for the multipart framing and text fields around it.
pub(crate) const PUBLISH_BODY_LIMIT: DefaultBodyLimit =
    DefaultBodyLimit::max(MAX_PAGE_BYTES + 1024 * 1024);

/// Publish a new version of an Artifact from a multipart upload.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/artifacts",
    tag = "artifacts",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    request_body(content_type = "multipart/form-data", content = PublishArtifactBody),
    responses(
        (status = 200, description = "Version published", body = ArtifactResponse),
        (status = 400, description = "Invalid request", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Caller cannot change the Task", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 413, description = "Page too large", body = moatless_api_schemas::error::ErrorBody),
        (status = 503, description = "Object storage not configured", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn publish_artifact(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    multipart: Multipart,
) -> Result<Json<ArtifactResponse>, ArtifactError> {
    if !artifacts.is_configured() {
        return Err(ArtifactError::StorageNotConfigured);
    }
    let req = read_publish(multipart).await?;
    let response = super::service::publish(
        tasks.as_ref(),
        artifacts.as_ref(),
        &auth.scope_context(),
        &task_id,
        req,
    )
    .await?;
    Ok(Json(response))
}

/// List a Task's published Artifacts, most recently updated first.
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/artifacts",
    tag = "artifacts",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Artifact list", body = ListArtifactsResponse),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_task_artifacts(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
) -> Result<Json<ListArtifactsResponse>, ArtifactError> {
    let items = super::service::list(
        tasks.as_ref(),
        artifacts.as_ref(),
        &auth.scope_context(),
        &task_id,
    )
    .await?;
    Ok(Json(ListArtifactsResponse { items }))
}

/// Query parameters of [`serve_artifact_page`] and [`serve_latest_artifact_page`].
#[derive(Debug, Default, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ServePageQuery {
    /// Any value serves the page as published, as a download.
    pub download: Option<String>,
    /// Any value serves the page itself to a browser opening the URL, rather
    /// than the viewer with Moatless's bar above it.
    pub raw: Option<String>,
}

/// Serve the page of a published Artifact version.
#[utoipa::path(
    get,
    path = "/api/v1/artifacts/{artifact_id}/{version}",
    tag = "artifacts",
    params(
        ("artifact_id" = String, Path, description = "Artifact identifier"),
        ("version" = String, Path, description = "Version, as in `v3`"),
        ServePageQuery
    ),
    responses(
        (status = 200, description = "The HTML page", body = String, content_type = "text/html"),
        (status = 404, description = "Artifact or version not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn serve_artifact_page(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    State(config): State<Arc<Config>>,
    auth: AuthUser,
    headers: HeaderMap,
    Path((artifact_id, version)): Path<(String, String)>,
    Query(query): Query<ServePageQuery>,
) -> Result<Response, ArtifactError> {
    if super::shell::wanted(&headers, query.raw.is_some(), query.download.is_some()) {
        let view = super::service::page_view(
            tasks.as_ref(),
            artifacts.as_ref(),
            &auth.scope_context(),
            &artifact_id,
            Some(&version),
        )
        .await?;
        return Ok(shell_response(&view, false, &config));
    }
    let page = super::service::read_page(
        tasks.as_ref(),
        artifacts.as_ref(),
        &auth.scope_context(),
        &artifact_id,
        Some(&version),
    )
    .await?;
    Ok(page_response(&artifacts, &auth, page, &query, false))
}

/// Serve the page of the Artifact's latest version at its unversioned URL.
#[utoipa::path(
    get,
    path = "/api/v1/artifacts/{artifact_id}",
    tag = "artifacts",
    params(
        ("artifact_id" = String, Path, description = "Artifact identifier"),
        ServePageQuery
    ),
    responses(
        (status = 200, description = "The latest version's HTML page", body = String, content_type = "text/html"),
        (status = 404, description = "Artifact not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn serve_latest_artifact_page(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    State(config): State<Arc<Config>>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(artifact_id): Path<String>,
    Query(query): Query<ServePageQuery>,
) -> Result<Response, ArtifactError> {
    if super::shell::wanted(&headers, query.raw.is_some(), query.download.is_some()) {
        let view = super::service::page_view(
            tasks.as_ref(),
            artifacts.as_ref(),
            &auth.scope_context(),
            &artifact_id,
            None,
        )
        .await?;
        return Ok(shell_response(&view, true, &config));
    }
    let page = super::service::read_page(
        tasks.as_ref(),
        artifacts.as_ref(),
        &auth.scope_context(),
        &artifact_id,
        None,
    )
    .await?;
    Ok(page_response(&artifacts, &auth, page, &query, true))
}

/// The response serving `page`. `latest` marks the unversioned URL.
fn page_response(
    artifacts: &Artifacts,
    auth: &AuthUser,
    page: super::service::Page,
    query: &ServePageQuery,
    latest: bool,
) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(PAGE_CONTENT_TYPE),
    );
    let csp = page_csp(artifacts.public_url());
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::try_from(csp).unwrap_or(HeaderValue::from_static(PAGE_CSP)),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    let token = if query.download.is_none() {
        page_token(artifacts, auth, &page)
    } else {
        None
    };
    let download = query.download.is_some();
    let cache = if download {
        "no-store"
    } else {
        page_cache(page.inline, latest, token.is_some())
    };
    let body = if download {
        let disposition = format!("attachment; filename=\"{}\"", download_name(&page.title));
        if let Ok(value) = HeaderValue::try_from(disposition) {
            headers.insert(header::CONTENT_DISPOSITION, value);
        }
        page.html
    } else if page.inline {
        super::bootstrap::inject(&page.html)
    } else {
        page.html
    };
    let body = match token {
        Some(token) => super::bootstrap::inject_data_script(
            &body,
            &super::bootstrap::DataScriptConfig {
                token: &token,
                artifact_id: &page.artifact_id,
                viewer: &auth.login,
                viewer_id: &auth.scope_context().user_id,
                rules: page.record_rules.as_ref(),
                collections: page
                    .collections
                    .iter()
                    .map(super::bootstrap::PageCollection::from)
                    .collect(),
            },
        ),
        None => body,
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    // A URL answers the viewer or the page by this header, so a frame's cached
    // page must not answer a later navigation to the same URL.
    headers.insert(header::VARY, HeaderValue::from_static("Sec-Fetch-Dest"));
    (headers, body).into_response()
}

/// The viewer for `view`; `latest` marks the unversioned URL.
fn shell_response(view: &super::service::PageView, latest: bool, config: &Config) -> Response {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let frontend_url = config.auth.frontend_url.trim_end_matches('/');
    let body = super::shell::render(view, latest, frontend_url, &nonce);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(PAGE_CONTENT_TYPE),
    );
    if let Ok(csp) = HeaderValue::try_from(super::shell::csp(&nonce)) {
        headers.insert(header::CONTENT_SECURITY_POLICY, csp);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // What the viewer offers depends on the reader, and its nonce on the load.
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(header::VARY, HeaderValue::from_static("Sec-Fetch-Dest"));
    (headers, body).into_response()
}

/// Set whether any signed-in user may read an Artifact, as a user who can
/// change its Task.
#[utoipa::path(
    put,
    path = "/api/v1/artifacts/{artifact_id}/visibility",
    tag = "artifacts",
    params(
        ("artifact_id" = String, Path, description = "Artifact identifier")
    ),
    request_body = SetArtifactVisibilityRequest,
    responses(
        (status = 200, description = "Visibility set", body = ArtifactResponse),
        (status = 403, description = "Caller cannot change the Task", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Artifact not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn set_artifact_visibility(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    auth: AuthUser,
    Path(artifact_id): Path<String>,
    Json(req): Json<SetArtifactVisibilityRequest>,
) -> Result<Json<ArtifactResponse>, ArtifactError> {
    let response = super::service::set_visibility(
        tasks.as_ref(),
        artifacts.as_ref(),
        &auth.scope_context(),
        &artifact_id,
        req.visibility,
    )
    .await?;
    Ok(Json(response))
}

/// Fork a published Artifact version into a new Task, as a private Artifact
/// of its own there.
#[utoipa::path(
    post,
    path = "/api/v1/artifacts/{artifact_id}/{version}/fork",
    tag = "artifacts",
    params(
        ("artifact_id" = String, Path, description = "Artifact identifier"),
        ("version" = String, Path, description = "Version, as in `v3`")
    ),
    request_body = ForkArtifactRequest,
    responses(
        (status = 200, description = "Fork created", body = ForkArtifactResponse),
        (status = 400, description = "Invalid request", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Artifact, version or Workspace not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 409, description = "The version's turn is still in flight", body = moatless_api_schemas::error::ErrorBody),
        (status = 503, description = "Object storage not configured", body = moatless_api_schemas::error::ErrorBody)
    )
)]
#[expect(
    clippy::too_many_arguments,
    reason = "the ports of `task::handlers::fork_task`, which a fork of the Task goes through, plus the Artifact store the copy is written to; no caller above a handler exists to hoist them to"
)]
pub async fn fork_artifact(
    State(workspaces): State<Arc<dyn WorkspaceResolver>>,
    State(repositories): State<Arc<dyn RepositoryReader>>,
    State(tasks): State<Arc<TaskCommands>>,
    State(users): State<Arc<dyn UserResolver>>,
    State(credential_sync): State<Arc<CredentialSync>>,
    State(secrets): State<Arc<dyn SecretCatalog>>,
    State(config): State<Arc<Config>>,
    State(task_creation): State<Arc<TaskCreationService>>,
    State(task_fork): State<Arc<TaskForkService>>,
    State(task_switch): State<Arc<TaskSwitchService>>,
    State(message_send): State<Arc<MessageSendService>>,
    State(task_presenter): State<Arc<TaskPresenter>>,
    State(artifacts): State<Arc<Artifacts>>,
    auth: AuthUser,
    Path((artifact_id, version)): Path<(String, String)>,
    Json(req): Json<ForkArtifactRequest>,
) -> Result<Json<ForkArtifactResponse>, ArtifactError> {
    let user_id = auth.user_id.to_string();
    let outcome = super::fork::fork(
        super::fork::ArtifactForkPorts {
            create: ForkCreationPorts {
                normalize: CreateTaskPorts {
                    workspaces: workspaces.as_ref(),
                    repositories: repositories.as_ref(),
                    tasks: tasks.as_ref(),
                    users: users.as_ref(),
                    credential_sync: &credential_sync,
                    secrets: secrets.as_ref(),
                    config: &config,
                },
                task_creation: &task_creation,
                message_send,
                switch: &task_switch,
            },
            task_fork: &task_fork,
        },
        artifacts.as_ref(),
        &auth.scope_context(),
        &artifact_id,
        &version,
        req,
        move |task| async move {
            match task_presenter.enrich_task(&task, &user_id).await {
                Ok(resp) => serde_json::json!({ "task": resp }),
                Err(e) => {
                    tracing::warn!(task_id = %task.task_id, error = %e, "Failed to enrich an artifact fork's task for its event");
                    serde_json::json!({ "task": { "taskId": task.task_id } })
                }
            }
        },
    )
    .await?;
    let frontend_url = config.auth.frontend_url.trim_end_matches('/');
    Ok(Json(ForkArtifactResponse {
        task_url: format!("{frontend_url}/tasks/{}", outcome.task.task_id),
        task_id: outcome.task.task_id,
        artifact: outcome.artifact,
    }))
}

/// The `Cache-Control` value for a page served for viewing, not download.
fn page_cache(inline: bool, latest: bool, has_token: bool) -> &'static str {
    if has_token {
        // The token names this reader, so neither a shared nor a later load
        // may reuse the page that carries it.
        "private, no-store"
    } else if latest {
        // The unversioned URL's page changes with each publish.
        "private, no-cache"
    } else if inline {
        // Not immutable: the injected bootstrap changes with a t3code upgrade.
        "private, max-age=3600"
    } else {
        "private, max-age=31536000, immutable"
    }
}

/// A page token for `auth` on `page`, or `None` when the page neither accepts
/// data nor reads a collection, or this deployment has no signing key. A signing failure serves the page
/// without one, so its saves fail rather than the page.
fn page_token(
    artifacts: &Artifacts,
    auth: &AuthUser,
    page: &super::service::Page,
) -> Option<String> {
    let key = artifacts
        .signing_key()
        .filter(|_| page.accepts_data || !page.collections.is_empty())?;
    let scope = auth.scope_context();
    super::page_token::mint(
        key,
        &scope.user_id,
        scope.is_admin,
        &auth.login,
        &page.artifact_id,
    )
    .map_err(|e| {
        tracing::warn!(
            error = %e,
            artifact_id = %page.artifact_id,
            "Failed to sign an artifact page token"
        );
    })
    .ok()
}

/// The page's policy, less the backend origin [`page_csp`] adds. No
/// `allow-same-origin`, so the page runs in an opaque origin, where `'self'`
/// still matches the serving origin over `https:` and `wss:` in Chromium and
/// Firefox. `default-src` does not cover `form-action`, and `allow-forms` would
/// let a submit send the page's data to any host.
const PAGE_CSP: &str = "sandbox allow-scripts allow-forms allow-popups; frame-ancestors 'self'; \
    default-src 'none'; base-uri 'none'; form-action 'none'; \
    script-src 'unsafe-inline' https://cdn.jsdelivr.net; \
    style-src 'unsafe-inline' https://fonts.googleapis.com https://cdn.jsdelivr.net; \
    font-src https://fonts.gstatic.com https://cdn.jsdelivr.net data:; \
    img-src data: blob:; connect-src 'self'";

/// [`PAGE_CSP`] with `public_url` and its socket origin added to
/// `connect-src`, for a browser whose `'self'` does not match the serving
/// origin inside the sandbox.
fn page_csp(public_url: &str) -> String {
    let socket = if let Some(host) = public_url.strip_prefix("https://") {
        format!("wss://{host}")
    } else if let Some(host) = public_url.strip_prefix("http://") {
        format!("ws://{host}")
    } else {
        return PAGE_CSP.to_string();
    };
    format!("{PAGE_CSP} {public_url} {socket}")
}

/// `title` reduced to a file name that needs no quoting in a header.
fn download_name(title: &str) -> String {
    let stem: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let stem = stem.trim_matches('-');
    format!("{}.html", if stem.is_empty() { "artifact" } else { stem })
}

/// Collect the multipart form, refusing it as soon as the page passes
/// [`MAX_PAGE_BYTES`] rather than after buffering all of it.
async fn read_publish(mut multipart: Multipart) -> Result<PublishRequest, ArtifactError> {
    let mut req = PublishRequest::default();
    let mut inline_fields = InlineRequest::default();
    while let Some(mut field) = multipart.next_field().await.map_err(multipart_error)? {
        match field.name().unwrap_or_default() {
            "title" => req.title = field.text().await.map_err(multipart_error)?,
            "slug" => req.slug = Some(field.text().await.map_err(multipart_error)?),
            "data" => {
                req.data.accepts_data = field.text().await.map_err(multipart_error)? == "true";
            }
            "notify" => {
                req.data.notify_on_data = field.text().await.map_err(multipart_error)? == "true";
            }
            "rules" => {
                let text = field.text().await.map_err(multipart_error)?;
                req.data.rules =
                    Some(RecordRules::parse(&text).map_err(ArtifactError::BadRequest)?);
            }
            "collection" => {
                req.collections
                    .push(field.text().await.map_err(multipart_error)?);
            }
            "visibility" => {
                let value = field.text().await.map_err(multipart_error)?;
                req.visibility =
                    Some(Visibility::try_from(value.as_str()).map_err(ArtifactError::BadRequest)?);
            }
            "inline" => {
                let value = field.text().await.map_err(multipart_error)?;
                if value == "true" {
                    req.inline.get_or_insert_with(InlineRequest::default);
                }
            }
            "height" => {
                let value = field.text().await.map_err(multipart_error)?;
                let height = value.trim().parse().map_err(|_| {
                    ArtifactError::BadRequest(format!("height {value:?} is not a number"))
                })?;
                inline_fields.height = Some(height);
            }
            "file" => {
                if req.file_name.is_some() {
                    return Err(ArtifactError::BadRequest(
                        "an artifact is a single HTML file; send one `file` part".to_string(),
                    ));
                }
                let name = field.file_name().unwrap_or_default().to_string();
                while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
                    if req.html.len() + chunk.len() > MAX_PAGE_BYTES {
                        return Err(ArtifactError::PayloadTooLarge(format!(
                            "{name} exceeds {MAX_PAGE_BYTES} bytes"
                        )));
                    }
                    req.html.extend_from_slice(&chunk);
                }
                req.file_name = Some(name);
            }
            other => {
                return Err(ArtifactError::BadRequest(format!(
                    "unexpected form field {other:?}"
                )));
            }
        }
    }
    // `height` means nothing without `inline`, in either order.
    if let Some(inline) = req.inline.as_mut() {
        *inline = inline_fields;
    }
    Ok(req)
}

fn multipart_error(e: MultipartError) -> ArtifactError {
    if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ArtifactError::PayloadTooLarge(e.body_text())
    } else {
        ArtifactError::BadRequest(format!("multipart error: {}", e.body_text()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directive<'a>(csp: &'a str, name: &str) -> &'a str {
        csp.split(';')
            .map(str::trim)
            .find(|d| d.split_whitespace().next() == Some(name))
            .unwrap_or_else(|| panic!("no {name} in {csp}"))
    }

    #[test]
    fn page_cache_revalidates_the_latest_url_and_caches_a_version() {
        assert_eq!(page_cache(false, true, false), "private, no-cache");
        assert_eq!(page_cache(true, true, false), "private, no-cache");
        assert_eq!(
            page_cache(false, false, false),
            "private, max-age=31536000, immutable"
        );
        assert_eq!(page_cache(true, false, false), "private, max-age=3600");
        assert_eq!(page_cache(false, true, true), "private, no-store");
    }

    #[test]
    fn the_page_policy_allows_only_the_skill_hosts_and_the_backend() {
        let csp = page_csp("https://moatless.example.com");
        assert_eq!(
            directive(&csp, "sandbox"),
            "sandbox allow-scripts allow-forms allow-popups"
        );
        assert_eq!(directive(&csp, "default-src"), "default-src 'none'");
        assert_eq!(
            directive(&csp, "script-src"),
            "script-src 'unsafe-inline' https://cdn.jsdelivr.net"
        );
        assert_eq!(directive(&csp, "img-src"), "img-src data: blob:");
        assert_eq!(directive(&csp, "form-action"), "form-action 'none'");
        assert_eq!(
            directive(&csp, "connect-src"),
            "connect-src 'self' https://moatless.example.com wss://moatless.example.com"
        );
    }

    #[test]
    fn a_plain_http_backend_is_reached_over_ws() {
        let csp = page_csp("http://localhost:8080");
        assert_eq!(
            directive(&csp, "connect-src"),
            "connect-src 'self' http://localhost:8080 ws://localhost:8080"
        );
    }

    #[test]
    fn a_public_url_without_a_scheme_leaves_only_self() {
        assert_eq!(
            directive(&page_csp("moatless"), "connect-src"),
            "connect-src 'self'"
        );
        assert!(HeaderValue::try_from(page_csp("https://moatless.example.com")).is_ok());
    }
}
