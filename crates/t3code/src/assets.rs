//! The bytes behind `assets.createUrl`: one file from a conversation's
//! checkout, served over HTTP so a browser can render it, and one chat
//! attachment, addressed at the upload route that already serves it.
//!
//! T3 asks for a *URL* rather than for content because it points a frame at the
//! answer. Three properties of that are deliberate.
//!
//! **The URL carries no credential.** The frame that loads it is same-site, so
//! the session cookie is sent and the auth middleware has run by the time
//! [`serve`] is entered. [`serve`] — not [`create_url`] — checks the viewer
//! against the Task, through the same follow-and-access read every other file
//! surface uses: the URL is an address that still has to be earned on use, not
//! a capability. A client authenticating by API key cannot use these URLs at
//! all, because a frame navigation sends no headers.
//!
//! **The document is served into a sandbox.** Behind these URLs is whatever an
//! agent wrote, in a frame on the application's own origin.
//! `Content-Security-Policy: sandbox` drops it into an opaque origin: it still
//! renders and loads its own stylesheets and scripts, but it is no longer
//! same-origin with anything and the cookie does not follow its requests.
//!
//! **The path in the URL is the path on disk.** A page's own `./style.css`
//! resolves against its URL and arrives here as the sibling path it is, which
//! is what makes the checkout-relative resolution [`crate::files`] does safe to
//! repeat here.

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use moatless_ports::sandbox::SandboxError;
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::checkout::Checkout;
use crate::workspace;
use crate::workspace::WorkspaceAccessError;
use crate::{UiRpcState, projection, viewer_scope};
use moatless_types::ui_rpc::UiRpcAuthIdentity;

/// Where [`create_url`]'s answers point, and what [`serve`] is mounted at.
pub(crate) const ASSET_ROUTE_PREFIX: &str = "/api/assets";
pub(crate) const ASSET_ROUTE_PATTERN: &str = "/api/assets/{thread_id}/{*relative_path}";

/// Where an attachment's bytes already live: the Moatless upload route, mounted
/// by the root backend rather than by this module. An attachment reaches this
/// deployment as an upload — see `backend/src/t3code/backend.rs` — so it is
/// addressed the way every other upload is, and nothing here needs to serve it.
const UPLOAD_ROUTE_PREFIX: &str = "/api/v1/uploads";

/// How long a minted URL is reported to be good for.
///
/// Nothing enforces it — the URL holds no credential and the route re-checks
/// the viewer on every request — so this is a re-mint interval and not a
/// lifetime. The contract requires the field, and the client re-asks well
/// inside it (`client-runtime/src/state/assets.ts:8`), so the value's only job
/// is to be far enough out that an open tab is not re-minting constantly.
const URL_TTL_MS: i64 = 60 * 60 * 1000;

/// What T3 previews in a browser frame, which is the only thing worth minting a
/// URL for. Mirrors `WORKSPACE_BROWSER_PREVIEW_EXTENSIONS` and
/// `WORKSPACE_IMAGE_PREVIEW_EXTENSIONS` in `packages/shared/src/filePreview.ts`.
///
/// Upstream admits video here for a `media-file` and this does not — see
/// `docs/deferred-changes.md`.
const PREVIEWABLE: [&str; 11] = [
    ".htm", ".html", ".pdf", ".avif", ".gif", ".ico", ".jpeg", ".jpg", ".png", ".svg", ".webp",
];

/// Why a resource has no URL to give.
///
/// Each is a tagged error the contract defines, and the client renders each
/// with its own sentence, so the distinction reaches the person: one says the
/// file is not there, one that it is not the kind of thing a browser frame
/// shows, and one that a project has no icon.
#[derive(Debug)]
pub(crate) enum AssetUrlError {
    NotFound,
    NotPreviewable,
    /// A project favicon, which a Moatless project never has — see
    /// [`create_url`].
    NoProjectFavicon,
}

impl AssetUrlError {
    pub(crate) fn tag(&self) -> &'static str {
        match self {
            Self::NotFound => "AssetWorkspaceAssetNotFoundError",
            Self::NotPreviewable => "AssetPreviewTypeValidationError",
            Self::NoProjectFavicon => "AssetProjectFaviconNotFoundError",
        }
    }
}

/// The URL a resource is reachable at, in the shape `AssetCreateUrlResult` has
/// on the wire.
///
/// `workspace-file` and `media-file` both name a file in the conversation's
/// checkout and both are served by [`serve`] below. The contract separates them
/// by where the path may point — a `media-file` path may be absolute and lie
/// outside the workspace — which on this fork is not a distinction: every read
/// goes through [`Checkout`], addressed inside the Sandbox, so the same path
/// handling answers both. An `attachment` is an ordinary Moatless upload and is
/// served by the upload route, the same address the Moatless UI renders it
/// from. A project favicon would be a file in a project's root directory, which
/// a Moatless project does not have, so that one answers
/// `AssetProjectFaviconNotFoundError` — which is what the client renders as a
/// project without an icon.
///
/// Nothing here reaches the Sandbox or the database. Whether the file exists is
/// settled by the request that fetches it, because that request has to look
/// anyway: a mint that proved existence first would double every read and still
/// be stale by the time the frame asked. The same is true of who may see it —
/// the upload route re-checks the viewer against the upload's own scope, so an
/// id minted here is no more reachable than the person asking already was.
pub(crate) fn create_url(resource: &Value) -> Result<Value, AssetUrlError> {
    match resource["_tag"].as_str() {
        Some("attachment") => attachment_url(resource),
        Some("workspace-file" | "media-file") => file_url(resource),
        Some("project-favicon") => Err(AssetUrlError::NoProjectFavicon),
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

/// An upload, addressed where uploads already are.
fn attachment_url(resource: &Value) -> Result<Value, AssetUrlError> {
    let Some(attachment_id) = resource["attachmentId"]
        .as_str()
        .filter(|id| !id.is_empty() && !id.contains('/'))
    else {
        return Err(AssetUrlError::NotFound);
    };
    Ok(json!({
        "relativeUrl": format!("{UPLOAD_ROUTE_PREFIX}/{}", escape(attachment_id)),
        "expiresAt": Utc::now().timestamp_millis() + URL_TTL_MS
    }))
}

/// A file in the conversation's checkout, whichever tag named it.
fn file_url(resource: &Value) -> Result<Value, AssetUrlError> {
    let (Some(thread_id), Some(path)) = (resource["threadId"].as_str(), resource["path"].as_str())
    else {
        return Err(AssetUrlError::NotFound);
    };

    // The client resolves a workspace path against the `cwd` it was given, so
    // what arrives is this module's own nominal worktree with the file hanging
    // off it. A path that is already relative is taken as it stands.
    let relative = path
        .strip_prefix(&projection::nominal_worktree(thread_id))
        .map_or(path, |nested| nested.trim_start_matches('/'));
    // What is minted has to be something [`serve`] can read, and it reads
    // through the Sandbox's own `validate_path` (`crates/sandbox/src/files`),
    // which refuses a leading `/` and any `..` before it looks at the disk.
    // Minting either answers the frame a 400 carrying that refusal instead of
    // the tag the client renders as a sentence — the same invisible refusal
    // this module exists to avoid. A `media-file` may name an absolute path
    // outside the workspace and this is where that stops: nothing outside the
    // checkout is reachable from here.
    if relative.is_empty() || relative.starts_with('/') || relative.contains("..") {
        return Err(AssetUrlError::NotFound);
    }
    let lowered = relative.to_ascii_lowercase();
    if !PREVIEWABLE.iter().any(|suffix| lowered.ends_with(suffix)) {
        return Err(AssetUrlError::NotPreviewable);
    }

    Ok(json!({
        "relativeUrl": asset_url(thread_id, relative),
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
/// Deliberately an allowlist: anything that is not plainly safe in a path
/// segment is escaped, which is stricter than it needs to be and never wrong.
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

/// One workspace file, as a browser wants it.
///
/// The viewer is checked here and not at mint time, because this is the request
/// that reads the file: [`workspace::addressed_conversation`] applies the same
/// follow and access filter the file tree and the editor go through, so this
/// route is exactly as wide as those and no wider.
pub(crate) async fn serve(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    Path((thread_id, relative_path)): Path<(String, String)>,
) -> Response {
    let viewer = viewer_scope(&identity);
    let cwd = projection::nominal_worktree(&thread_id);
    let task_id = match workspace::addressed_conversation(
        state.conversations.as_ref(),
        &viewer,
        &cwd,
    )
    .await
    {
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
        body,
    )
        .into_response()
}

/// What a browser is told when there is nothing to render.
///
/// A frame shows this text, so it is the same sentence the panels get from
/// [`WorkspaceAccessError::message`] rather than a second vocabulary for the
/// same failures.
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
/// The Sandbox reports a media type only for what it had to base64-encode, so
/// the text types a page is assembled from — its stylesheet, its script — are
/// named here. Getting `text/html` wrong is the difference between a rendered
/// page and a download prompt.
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

    /// A `media-file` path may name somewhere outside the workspace, and the
    /// Sandbox refuses a leading `/` or a `..` before it reads anything — so
    /// minting one would answer the frame a 400 carrying that refusal instead
    /// of the tag the client renders as a sentence.
    #[test]
    fn a_path_the_sandbox_would_refuse_is_refused_in_the_contracts_own_type() {
        let media = |path: &str| json!({ "_tag": "media-file", "threadId": "t-1", "path": path });
        for path in [
            "/etc/passwd.png",
            "/var/run/secrets/x.png",
            "../../etc/x.png",
            "docs/../../x.png",
        ] {
            assert!(
                matches!(create_url(&media(path)), Err(AssetUrlError::NotFound)),
                "{path} was minted, and the Sandbox would refuse it"
            );
        }
        // The same guard on the tag that has always been here: an absolute path
        // outside the checkout never resolved, it only failed later.
        assert!(matches!(
            create_url(&workspace_file("/etc/passwd.png")),
            Err(AssetUrlError::NotFound)
        ));
        // A path inside the checkout still mints, dots in a name and all.
        assert!(create_url(&media("/moatless/tasks/t-1/a.b.png")).is_ok());
    }

    /// An attachment is an upload, so it is addressed where uploads are — and
    /// nowhere near the workspace prefix, which is what the Sandbox serves.
    #[test]
    fn an_attachment_is_addressed_at_the_upload_route() {
        let minted =
            create_url(&json!({ "_tag": "attachment", "attachmentId": "a-1" })).expect("minted");
        assert_eq!(minted["relativeUrl"], "/api/v1/uploads/a-1");
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

    /// A Moatless project is a Workspace and has no root directory to hold an
    /// icon, so this keeps answering that there is none — but as the favicon
    /// error the contract defines, so the client renders "Project favicon was
    /// not found." and an operator can tell a project without an icon apart
    /// from a workspace file that is genuinely missing.
    #[test]
    fn a_project_favicon_has_no_url_and_says_so_as_a_favicon() {
        let refused = create_url(&json!({
            "_tag": "project-favicon", "cwd": "/moatless/workspaces/w-1"
        }))
        .expect_err("a Moatless project has no icon");
        assert!(matches!(refused, AssetUrlError::NoProjectFavicon));
        assert_eq!(refused.tag(), "AssetProjectFaviconNotFoundError");
    }

    /// The web client asks for a chat image, a video and any preview of a file
    /// outside the workspace root as a `media-file`
    /// (`apps/web/src/components/ChatMarkdown.tsx`,
    /// `components/files/FilePreviewPanel.tsx`), and this surface used to
    /// refuse every one of them: only `workspace-file` was matched, so an
    /// agent-written image in a thread never rendered. Both tags name a file in
    /// the same checkout here, so both mint the same URL.
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

    /// What [`serve`] does to a Sandbox's answer, against a Sandbox that really
    /// exists: the whole route rests on a text file arriving as text and an
    /// image arriving base64-encoded with a media type, and neither is
    /// something this side can check by reasoning about it.
    ///
    /// Ignored, because it needs a Sandbox. Point
    /// `T3_UI_RPC_TEST_SANDBOX_GRPC` at one — from inside a sandbox, its own
    /// server is at `http://127.0.0.1:50052` — and run:
    ///
    /// ```text
    /// T3_UI_RPC_TEST_SANDBOX_GRPC=http://127.0.0.1:50052 \
    ///   cargo test -p moatless-t3code --features live-sandbox-tests assets -- --ignored --nocapture
    /// ```
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
