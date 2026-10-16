//! The bytes behind `assets.createUrl`: one file from a conversation's
//! checkout, served over HTTP, and one chat attachment served by the existing
//! upload route.
//!
//! The minted URL carries no credential — [`serve`] re-checks the viewer on
//! every request. It is served into a sandboxed origin
//! (`Content-Security-Policy: sandbox`), and the URL's path is the on-disk path
//! so relative links resolve.

use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use moatless_ports::sandbox::SandboxError;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::checkout::Checkout;
use crate::nominal_path;
use crate::workspace;
use crate::workspace::WorkspaceAccessError;
use crate::{UiRpcState, viewer_scope};
use moatless_types::ui_rpc::UiRpcAuthIdentity;

/// Where [`create_url`]'s answers point, and what [`serve`] is mounted at.
pub(crate) const ASSET_ROUTE_PREFIX: &str = "/api/assets";
pub(crate) const ASSET_ROUTE_PATTERN: &str = "/api/assets/{thread_id}/{*relative_path}";

/// Where an attachment's bytes already live: the Moatless upload route, mounted
/// by the root backend, not this module — see `backend/src/t3code/backend.rs`.
/// Nothing here serves it.
const UPLOAD_ROUTE_PREFIX: &str = "/api/v1/uploads";
const ARTIFACT_ROUTE_PREFIX: &str = "/api/v1/artifacts";

/// A project favicon's answer, since a Moatless project never has one.
///
/// The client reads the last segment against `PROJECT_FAVICON_FALLBACK_MARKER`
/// and keeps "no icon" as an answer; deliberately not
/// `AssetProjectFaviconNotFoundError`, which it re-requests on every reconnect.
/// Nothing serves the URL, and the client never fetches it.
const PROJECT_FAVICON_MISSING_URL: &str = "/api/assets/project-favicon/project-favicon-missing";

/// How long a minted URL is reported to be good for — a re-mint interval, not
/// an enforced lifetime, since the URL holds no credential and the route
/// re-checks the viewer on every request.
const URL_TTL_MS: i64 = 60 * 60 * 1000;

/// What T3 previews in a browser frame — mirrors
/// `WORKSPACE_BROWSER_PREVIEW_EXTENSIONS` and
/// `WORKSPACE_IMAGE_PREVIEW_EXTENSIONS`, excluding video unlike upstream's
/// `media-file` handling.
const PREVIEWABLE: [&str; 11] = [
    ".htm", ".html", ".pdf", ".avif", ".gif", ".ico", ".jpeg", ".jpg", ".png", ".svg", ".webp",
];

/// Why a resource has no URL to give.
///
/// Each is a tagged error the contract defines; the client renders each with
/// its own sentence.
#[derive(Debug)]
pub(crate) enum AssetUrlError {
    NotFound,
    NotPreviewable,
}

impl AssetUrlError {
    pub(crate) fn tag(&self) -> &'static str {
        match self {
            Self::NotFound => "AssetWorkspaceAssetNotFoundError",
            Self::NotPreviewable => "AssetPreviewTypeValidationError",
        }
    }
}

/// The URL a resource is reachable at, as `AssetCreateUrlResult` on the wire.
///
/// `workspace-file`/`media-file` name a checkout file served by [`serve`];
/// `attachment` uses the upload route; existence and access are settled later,
/// by the request that fetches the file.
pub(crate) fn create_url(resource: &Value) -> Result<Value, AssetUrlError> {
    match resource["_tag"].as_str() {
        Some("attachment") => attachment_url(resource),
        Some("workspace-file" | "media-file") => file_url(resource),
        Some("project-favicon") => Ok(json!({
            "relativeUrl": PROJECT_FAVICON_MISSING_URL,
            "expiresAt": Utc::now().timestamp_millis() + URL_TTL_MS
        })),
        // `native-app-icon` and anything a newer client invents. Logged
        // because this arm is where a resource this surface has never served
        // lands, and answering every one of them "not found" is exactly how
        // `media-file` stayed invisible: a whole class of request refused
        // without a line anyone could grep.
        other => {
            warn!(
                resource = other.unwrap_or("(untagged)"),
                "Refused an asset resource this surface does not serve"
            );
            Err(AssetUrlError::NotFound)
        }
    }
}

/// An upload, addressed where uploads already are, or a shown Artifact
/// version's page, named as `html_render_item` names it.
fn attachment_url(resource: &Value) -> Result<Value, AssetUrlError> {
    let Some(attachment_id) = resource["attachmentId"]
        .as_str()
        .filter(|id| !id.is_empty() && !id.contains('/'))
    else {
        return Err(AssetUrlError::NotFound);
    };
    let relative_url = match artifact_version(attachment_id) {
        Some((artifact_id, version)) => {
            let download = if resource["disposition"] == "attachment" {
                "?download=1"
            } else {
                ""
            };
            format!(
                "{ARTIFACT_ROUTE_PREFIX}/{}/v{version}{download}",
                escape(artifact_id)
            )
        }
        None => format!("{UPLOAD_ROUTE_PREFIX}/{}", escape(attachment_id)),
    };
    Ok(json!({
        "relativeUrl": relative_url,
        "expiresAt": Utc::now().timestamp_millis() + URL_TTL_MS
    }))
}

/// The artifact id and version of an `artifact:<id>:v<n>` attachment id.
fn artifact_version(attachment_id: &str) -> Option<(&str, u32)> {
    let (artifact_id, version) = attachment_id.strip_prefix("artifact:")?.rsplit_once(":v")?;
    (!artifact_id.is_empty()).then_some(())?;
    Some((artifact_id, version.parse().ok()?))
}

/// A file in the conversation's checkout, whichever tag named it.
///
/// `disposition: "attachment"` mints a download of any file type; anything
/// else is a preview, limited to [`PREVIEWABLE`].
fn file_url(resource: &Value) -> Result<Value, AssetUrlError> {
    let (Some(thread_id), Some(path)) = (resource["threadId"].as_str(), resource["path"].as_str())
    else {
        return Err(AssetUrlError::NotFound);
    };

    // `path` is this module's nominal worktree plus what the agent wrote:
    // relative to a checkout, or absolute inside the Sandbox. Strip the
    // worktree prefix and leading `/` so it becomes a URL segment;
    // [`Checkout::resolve`] accepts a mention with or without the root.
    let relative = path
        .strip_prefix(&nominal_path::nominal_worktree(thread_id))
        .unwrap_or(path)
        .trim_start_matches('/');
    // Reject `..` here too (the Sandbox already does) so the client gets this
    // error type, not a raw status code. An absolute path passes through
    // uncaught — [`serve`] resolves it against the workspace listing, which
    // keeps anything outside the checkout unreachable.
    if relative.is_empty() || relative.contains("..") {
        return Err(AssetUrlError::NotFound);
    }
    let download = resource["disposition"] == "attachment";
    let lowered = relative.to_ascii_lowercase();
    if !download && !PREVIEWABLE.iter().any(|suffix| lowered.ends_with(suffix)) {
        return Err(AssetUrlError::NotPreviewable);
    }

    let mut url = asset_url(thread_id, relative);
    if download {
        url.push_str("?download=1");
    }
    Ok(json!({
        "relativeUrl": url,
        "expiresAt": Utc::now().timestamp_millis() + URL_TTL_MS
    }))
}

/// `/api/assets/{task}/{path}`, with every segment escaped and the separators
/// left alone — the path has to survive as a path, because a page's relative
/// links are resolved against it by the browser.
fn asset_url(task_id: &str, relative_path: &str) -> String {
    let mut url = format!("{ASSET_ROUTE_PREFIX}/{}", escape(task_id));
    for segment in relative_path.split('/') {
        url.push('/');
        url.push_str(&escape(segment));
    }
    url
}

/// Percent-escape one path segment.
///
/// Deliberately an allowlist: anything not plainly safe in a path segment is
/// escaped — stricter than necessary, never wrong.
fn escape(segment: &str) -> String {
    use std::fmt::Write;

    let mut escaped = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                escaped.push(byte as char);
            }
            _ => {
                let _ = write!(escaped, "%{byte:02X}");
            }
        }
    }
    escaped
}

/// The query string [`file_url`] appends to a download.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct AssetQuery {
    download: Option<String>,
}

/// One workspace file, as a browser wants it.
///
/// The viewer is checked here, not at mint time, through the same
/// follow-and-access filter the file tree and editor use.
pub(crate) async fn serve(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    Path((thread_id, relative_path)): Path<(String, String)>,
    Query(query): Query<AssetQuery>,
) -> Response {
    let viewer = viewer_scope(&identity);
    let cwd = nominal_path::nominal_worktree(&thread_id);
    let task_id =
        match workspace::addressed_conversation(state.reads.conversations(), &viewer, &cwd).await {
            Ok(task_id) => task_id,
            Err(error) => return refusal(&thread_id, &relative_path, &error),
        };

    let checkout = match Checkout::open(state.file_sources(), &task_id).await {
        Ok(checkout) => checkout,
        Err(error) => return refusal(&thread_id, &relative_path, &error),
    };

    let content = match checkout.read(&relative_path).await {
        Ok(content) => content,
        Err(error) => return refusal(&thread_id, &relative_path, &error),
    };

    let media_type = content_type(&relative_path, content.media_type.as_deref());
    let body = content.body.into_bytes();

    debug!(task_id = %task_id, path = %relative_path, bytes = body.len(), "Serving a workspace asset");
    let disposition = query
        .download
        .is_some()
        .then(|| attachment_disposition(&content.path));
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(media_type)),
            // The document is agent-written and this is the application's own
            // origin. No `allow-same-origin`, so it renders in an opaque one.
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("sandbox allow-scripts allow-forms allow-popups"),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            // A checkout changes under the person looking at it, and the URL
            // does not change with it.
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, no-store"),
            ),
        ],
        disposition.map(|value| [(header::CONTENT_DISPOSITION, value)]),
        body,
    )
        .into_response()
}

/// `Content-Disposition: attachment`, named after the file's last segment.
///
/// RFC 5987's `filename*`, because an agent names files in any script and a
/// quoted `filename` carries only ASCII; [`escape`]'s allowlist is a subset of
/// its `attr-char`.
fn attachment_disposition(path: &str) -> HeaderValue {
    let name = path.rsplit('/').next().unwrap_or(path);
    HeaderValue::from_str(&format!("attachment; filename*=UTF-8''{}", escape(name)))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

/// What a browser is told when there is nothing to render.
///
/// Uses the same sentence the panels get from
/// [`WorkspaceAccessError::message`], not a second vocabulary.
fn refusal(thread_id: &str, relative_path: &str, error: &WorkspaceAccessError) -> Response {
    let status = match error {
        WorkspaceAccessError::NoWorktree | WorkspaceAccessError::Unaddressable => {
            StatusCode::NOT_FOUND
        }
        WorkspaceAccessError::NoControl => StatusCode::FORBIDDEN,
        WorkspaceAccessError::NotRunning(_) => StatusCode::SERVICE_UNAVAILABLE,
        WorkspaceAccessError::Sandbox(SandboxError::NotFound(_)) => StatusCode::NOT_FOUND,
        WorkspaceAccessError::Sandbox(SandboxError::BadRequest(_)) => StatusCode::BAD_REQUEST,
        WorkspaceAccessError::Sandbox(_) => StatusCode::BAD_GATEWAY,
        WorkspaceAccessError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    debug!(
        thread_id = %thread_id,
        path = %relative_path,
        status = %status,
        "Refusing a workspace asset"
    );
    (status, error.message()).into_response()
}

/// What the browser is told this file is.
///
/// The Sandbox reports a media type only for what it base64-encoded, so text
/// types (stylesheet, script, html) are named here instead.
fn content_type(path: &str, sandbox_media_type: Option<&str>) -> &'static str {
    let extension = path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        "md" | "txt" => "text/plain; charset=utf-8",
        _ => match sandbox_media_type {
            Some("image/png") => "image/png",
            Some("image/jpeg") => "image/jpeg",
            Some("image/gif") => "image/gif",
            Some("image/webp") => "image/webp",
            Some("image/svg+xml") => "image/svg+xml",
            Some("image/x-icon") => "image/x-icon",
            Some("image/bmp") => "image/bmp",
            Some("application/pdf") => "application/pdf",
            Some("font/woff") => "font/woff",
            Some("font/woff2") => "font/woff2",
            Some("font/ttf") => "font/ttf",
            Some("font/otf") => "font/otf",
            // A type the Sandbox named that is not on this list is one no
            // browser frame renders, so it is offered as bytes rather than
            // echoed back as a header value this side never validated.
            Some(_) => "application/octet-stream",
            // Not binary and not a known text type: text is the safe reading,
            // and `nosniff` stops the browser deciding otherwise.
            None => "text/plain; charset=utf-8",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "live-sandbox-tests")]
    use base64::Engine;

    fn workspace_file(path: &str) -> Value {
        json!({ "_tag": "workspace-file", "threadId": "t-1", "path": path })
    }

    /// The client resolves the path against the `cwd` this module gave it, so
    /// what arrives is absolute and has to come back off.
    #[test]
    fn a_nominal_worktree_path_becomes_a_route_under_the_asset_prefix() {
        let minted = create_url(&workspace_file("/moatless/tasks/t-1/docs/report.html"))
            .expect("a previewable file");
        assert_eq!(minted["relativeUrl"], "/api/assets/t-1/docs/report.html");
        assert!(minted["expiresAt"].as_i64().is_some_and(|at| at > 0));
    }

    /// A page's own `./style.css` arrives as a sibling path, and the URL it is
    /// fetched from has to name that sibling — which is only true if every
    /// separator survives minting.
    #[test]
    fn a_path_keeps_its_separators_and_escapes_everything_else() {
        assert_eq!(asset_url("t-1", "a/b/c.html"), "/api/assets/t-1/a/b/c.html");
        assert_eq!(
            asset_url("t-1", "docs/my report.html"),
            "/api/assets/t-1/docs/my%20report.html"
        );
        assert_eq!(asset_url("t/1", "a.png"), "/api/assets/t%2F1/a.png");
    }

    #[test]
    fn only_what_a_browser_frame_renders_is_worth_a_url() {
        assert!(matches!(
            create_url(&workspace_file("/moatless/tasks/t-1/src/main.rs")),
            Err(AssetUrlError::NotPreviewable)
        ));
        assert!(create_url(&workspace_file("/moatless/tasks/t-1/a/b.PNG")).is_ok());
        assert!(matches!(
            create_url(&workspace_file("/moatless/tasks/t-1")),
            Err(AssetUrlError::NotFound)
        ));
    }

    /// A download is not a frame, so any file type is minted, and the route is
    /// told to answer it as an attachment.
    #[test]
    fn a_download_mints_any_file_type_with_the_download_flag() {
        let minted = create_url(&json!({
            "_tag": "workspace-file", "threadId": "t-1",
            "path": "/moatless/tasks/t-1/scratch/Appendix F.xlsx",
            "disposition": "attachment"
        }))
        .expect("a download of any file type");
        assert_eq!(
            minted["relativeUrl"],
            "/api/assets/t-1/scratch/Appendix%20F.xlsx?download=1"
        );
        assert!(matches!(
            create_url(&json!({
                "_tag": "workspace-file", "threadId": "t-1",
                "path": "/moatless/tasks/t-1/../etc/passwd",
                "disposition": "attachment"
            })),
            Err(AssetUrlError::NotFound)
        ));
    }

    #[test]
    fn an_attachment_is_named_after_the_last_segment_in_any_script() {
        assert_eq!(
            attachment_disposition("scratch/Appendix F.xlsx"),
            "attachment; filename*=UTF-8''Appendix%20F.xlsx"
        );
        assert_eq!(
            attachment_disposition("rapport-å.pdf"),
            "attachment; filename*=UTF-8''rapport-%C3%A5.pdf"
        );
    }

    /// A `..` resolves to nothing in [`Checkout`] and the Sandbox refuses it
    /// outright, so minting one would answer the frame a status code where the
    /// client renders a sentence.
    #[test]
    fn a_path_the_sandbox_would_refuse_is_refused_in_the_contracts_own_type() {
        let media = |path: &str| json!({ "_tag": "media-file", "threadId": "t-1", "path": path });
        for path in ["../../etc/x.png", "docs/../../x.png"] {
            assert!(
                matches!(create_url(&media(path)), Err(AssetUrlError::NotFound)),
                "{path} was minted, and the Sandbox would refuse it"
            );
        }
        // A path inside the checkout still mints, dots in a name and all.
        assert!(create_url(&media("/moatless/tasks/t-1/a.b.png")).is_ok());
    }

    /// An agent writes the absolute Sandbox path, and the client's
    /// `workspaceRoot` being the nominal worktree means it has no root to
    /// subtract, so the path passes through unresolved.
    #[test]
    fn an_absolute_sandbox_path_is_minted_for_serve_to_resolve() {
        let minted = create_url(&workspace_file(
            "/opt/moatless/workspace/moatless/docs/shot.png",
        ))
        .expect("a previewable file");
        assert_eq!(
            minted["relativeUrl"], "/api/assets/t-1/opt/moatless/workspace/moatless/docs/shot.png",
            "the root becomes a segment: a URL with `//` in it is one a proxy may merge"
        );
    }

    /// An attachment is an upload, so it is addressed where uploads are — and
    /// nowhere near the workspace prefix, which is what the Sandbox serves.
    #[test]
    fn an_attachment_is_addressed_at_the_upload_route() {
        let minted =
            create_url(&json!({ "_tag": "attachment", "attachmentId": "a-1" })).expect("minted");
        assert_eq!(minted["relativeUrl"], "/api/v1/uploads/a-1");
    }

    /// A shown Artifact's page loads from the version's own route, which serves
    /// it with the bootstrap; a download asks that route for the raw page.
    #[test]
    fn an_artifact_attachment_is_addressed_at_its_version() {
        let id = "artifact:4b1c:v3";
        let minted = create_url(&json!({ "_tag": "attachment", "attachmentId": id })).unwrap();
        assert_eq!(minted["relativeUrl"], "/api/v1/artifacts/4b1c/v3");
        let download = create_url(
            &json!({ "_tag": "attachment", "attachmentId": id, "disposition": "attachment" }),
        )
        .unwrap();
        assert_eq!(
            download["relativeUrl"],
            "/api/v1/artifacts/4b1c/v3?download=1"
        );
        let malformed =
            create_url(&json!({ "_tag": "attachment", "attachmentId": "artifact:4b1c:vx" }))
                .unwrap();
        assert!(
            malformed["relativeUrl"]
                .as_str()
                .unwrap()
                .starts_with("/api/v1/uploads/")
        );
    }

    /// An id that would leave the route it is a segment of cannot be one.
    #[test]
    fn an_attachment_id_that_is_a_path_has_no_url() {
        assert!(matches!(
            create_url(&json!({ "_tag": "attachment", "attachmentId": "../secrets" })),
            Err(AssetUrlError::NotFound)
        ));
        assert!(matches!(
            create_url(&json!({ "_tag": "attachment", "attachmentId": "" })),
            Err(AssetUrlError::NotFound)
        ));
    }

    /// A Moatless project has no root directory to hold an icon, so this
    /// answers with the URL the client reads as "this project has none".
    #[test]
    fn a_project_favicon_answers_the_missing_marker() {
        let minted = create_url(&json!({
            "_tag": "project-favicon", "cwd": "/moatless/workspaces/w-1"
        }))
        .expect("a missing icon is an answer, not a refusal");
        let url = minted["relativeUrl"].as_str().expect("a URL");
        assert_eq!(url.rsplit('/').next(), Some("project-favicon-missing"));
    }

    /// The web client asks for a chat image, a video, or a file outside the
    /// workspace root as a `media-file`. Both tags name a file in the same
    /// checkout here, so both mint the same URL.
    #[test]
    fn a_media_file_is_minted_the_same_way_a_workspace_file_is() {
        let media = json!({
            "_tag": "media-file", "threadId": "t-1",
            "path": "/moatless/tasks/t-1/docs/shot.png"
        });
        let minted = create_url(&media).expect("a previewable file");
        assert_eq!(minted["relativeUrl"], "/api/assets/t-1/docs/shot.png");
        assert_eq!(
            minted["relativeUrl"],
            create_url(&workspace_file("/moatless/tasks/t-1/docs/shot.png"))
                .expect("the same file")["relativeUrl"]
        );
    }

    /// [`serve`] reads a whole file into memory and answers no range request,
    /// so a video is refused as a preview type rather than read wholesale to be
    /// scrubbed — see [`PREVIEWABLE`].
    #[test]
    fn a_video_is_refused_as_a_preview_type_rather_than_served_whole() {
        assert!(matches!(
            create_url(&json!({
                "_tag": "media-file", "threadId": "t-1",
                "path": "/moatless/tasks/t-1/demo.mp4"
            })),
            Err(AssetUrlError::NotPreviewable)
        ));
    }

    /// A page that arrives as `text/plain` is a page nobody sees, so the text
    /// types are named here rather than left to the Sandbox, which reports one
    /// only for what it base64-encoded.
    #[test]
    fn a_page_and_the_files_it_is_assembled_from_are_named_as_themselves() {
        assert_eq!(content_type("a/b.html", None), "text/html; charset=utf-8");
        assert_eq!(content_type("a/b.css", None), "text/css; charset=utf-8");
        assert_eq!(
            content_type("a/b.js", None),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type("a/b.png", Some("image/png")), "image/png");
        assert_eq!(content_type("a/b.bin", None), "text/plain; charset=utf-8");
        assert_eq!(
            content_type("a/b.bin", Some("application/x-tar")),
            "application/octet-stream"
        );
    }

    /// Exercises [`serve`] against a real Sandbox; point
    /// `T3_UI_RPC_TEST_SANDBOX_GRPC` at its gRPC endpoint and run with
    /// `--features live-sandbox-tests -- --ignored --nocapture`.
    #[cfg(feature = "live-sandbox-tests")]
    #[tokio::test]
    #[ignore = "needs a Sandbox at T3_UI_RPC_TEST_SANDBOX_GRPC"]
    async fn a_real_sandbox_answers_a_page_as_text_and_an_image_as_bytes() {
        let url = std::env::var("T3_UI_RPC_TEST_SANDBOX_GRPC")
            .expect("point T3_UI_RPC_TEST_SANDBOX_GRPC at a Sandbox's gRPC endpoint");
        let client = sandbox_proto::client::SandboxClient::connect(&url).expect("a client");
        let listing = client.list_files(".", true, true).await.expect("a listing");

        for (extension, expected) in [(".html", "text/html; charset=utf-8"), (".png", "image/png")]
        {
            let Some(path) = listing
                .iter()
                .find(|path| path.to_ascii_lowercase().ends_with(extension))
            else {
                println!("no {extension} in this workspace, skipping");
                continue;
            };
            let file = client.read_file(path, false).await.expect("the file");
            let served = content_type(path, file.media_type.as_deref());
            assert_eq!(served, expected, "{path} would be served as {served}");

            let bytes = if file.is_binary.unwrap_or(false) {
                base64::engine::general_purpose::STANDARD
                    .decode(&file.content)
                    .expect("the Sandbox base64-encodes what it cannot decode")
            } else {
                file.content.into_bytes()
            };
            assert!(!bytes.is_empty(), "{path} served no bytes");
            println!("{path} -> {served}, {} bytes", bytes.len());
        }
    }
}
