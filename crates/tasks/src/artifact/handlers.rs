//! Artifact HTTP handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::Multipart;
use axum_extra::extract::multipart::MultipartError;

use crate::task::operations::TaskCommands;
use moatless_core::http_auth::AuthUser;
use moatless_types::task::Visibility;

use super::error::ArtifactError;
use super::models::{InlineRequest, PublishRequest};
use super::operations::{Artifacts, PAGE_CONTENT_TYPE};
use super::rules::RecordRules;
use super::schemas::{ArtifactResponse, ListArtifactsResponse, PublishArtifactBody};
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

/// Redirect to the page of the Artifact's latest version.
#[utoipa::path(
    get,
    path = "/api/v1/artifacts/{artifact_id}",
    tag = "artifacts",
    params(
        ("artifact_id" = String, Path, description = "Artifact identifier")
    ),
    responses(
        (status = 302, description = "Redirect to the latest version"),
        (status = 404, description = "Artifact not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn open_latest_artifact(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    auth: AuthUser,
    Path(artifact_id): Path<String>,
) -> Result<Response, ArtifactError> {
    let location = super::service::latest_location(
        tasks.as_ref(),
        artifacts.as_ref(),
        &auth.scope_context(),
        &artifact_id,
    )
    .await?;
    let location = HeaderValue::try_from(location)
        .map_err(|_| ArtifactError::NotFound(artifact_id.clone()))?;
    Ok((
        StatusCode::FOUND,
        [
            (header::LOCATION, location),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
    )
        .into_response())
}

/// Query parameters of [`serve_artifact_page`].
#[derive(Debug, Default, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ServePageQuery {
    /// Any value serves the page as published, as a download.
    pub download: Option<String>,
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
    auth: AuthUser,
    Path((artifact_id, version)): Path<(String, String)>,
    Query(query): Query<ServePageQuery>,
) -> Result<Response, ArtifactError> {
    let page = super::service::read_page(
        tasks.as_ref(),
        artifacts.as_ref(),
        &auth.scope_context(),
        &artifact_id,
        &version,
    )
    .await?;
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
        page_token(&artifacts, &auth, &page)
    } else {
        None
    };
    let (cache, body) = if query.download.is_some() {
        let disposition = format!("attachment; filename=\"{}\"", download_name(&page.title));
        if let Ok(value) = HeaderValue::try_from(disposition) {
            headers.insert(header::CONTENT_DISPOSITION, value);
        }
        ("no-store", page.html)
    } else if page.inline {
        // Not immutable: the injected bootstrap changes with a t3code upgrade.
        (
            "private, max-age=3600",
            super::bootstrap::inject(&page.html),
        )
    } else {
        ("private, max-age=31536000, immutable", page.html)
    };
    // The token names this reader, so neither a shared nor a later load may
    // reuse the page that carries it.
    let (cache, body) = match token {
        Some(token) => (
            "private, no-store",
            super::bootstrap::inject_data_script(
                &body,
                &super::bootstrap::DataScriptConfig {
                    token: &token,
                    artifact_id: &page.artifact_id,
                    viewer: &auth.login,
                    viewer_id: &auth.scope_context().user_id,
                    rules: page.record_rules.as_ref(),
                },
            ),
        ),
        None => (cache, body),
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    Ok((headers, body).into_response())
}

/// A page token for `auth` on `page`, or `None` when the page accepts no data
/// or this deployment has no signing key. A signing failure serves the page
/// without one, so its saves fail rather than the page.
fn page_token(
    artifacts: &Artifacts,
    auth: &AuthUser,
    page: &super::service::Page,
) -> Option<String> {
    let key = artifacts.signing_key().filter(|_| page.accepts_data)?;
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
