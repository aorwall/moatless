//! The git smart-HTTP routes, streamed in both directions.

use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::{StreamExt, TryStreamExt};
use serde::Deserialize;
use tracing::{info, warn};

use crate::git_proxy::body::WatchedBody;
use crate::git_proxy::push_watch::PushWatch;
use crate::git_proxy::service::{GitProxy, GitProxyError, Upstream};
use crate::github_policy::push::{self, Commands, Parsed};
use crate::github_policy::{self, Refusal, Request as PolicyRequest};

/// The two git services this proxy forwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitService {
    UploadPack,
    ReceivePack,
}

impl GitService {
    /// The name git puts in `?service=` and in the RPC path.
    fn name(self) -> &'static str {
        match self {
            Self::UploadPack => "git-upload-pack",
            Self::ReceivePack => "git-receive-pack",
        }
    }

    /// How long the upstream body may stay silent before the proxy logs it.
    /// `None` for receive-pack, whose host may check a push for longer.
    fn quiet_limit(self) -> Option<Duration> {
        match self {
            Self::UploadPack => Some(Duration::from_secs(20)),
            Self::ReceivePack => None,
        }
    }
}

/// The service `name` asks for, or `None` for one this proxy does not forward.
///
/// Forwarding anything else would make the proxy a general-purpose fetcher for
/// whatever the credential can reach.
fn service_named(name: &str) -> Option<GitService> {
    match name {
        "git-upload-pack" => Some(GitService::UploadPack),
        "git-receive-pack" => Some(GitService::ReceivePack),
        _ => None,
    }
}

/// Headers copied from git's request onto the upstream one.
///
/// `Content-Encoding` is required: an unlabeled gzipped `upload-pack` request
/// reads as pkt-line upstream, and git fails with `fatal: expected 'packfile'`.
const FORWARDED_REQUEST_HEADERS: [HeaderName; 4] = [
    header::CONTENT_TYPE,
    header::CONTENT_ENCODING,
    header::ACCEPT,
    header::USER_AGENT,
];

/// Headers copied from the upstream response.
///
/// `Content-Length` and `Transfer-Encoding` stay out: `reqwest` already
/// dechunked the body, so forwarding either would describe an encoding the
/// bytes no longer carry.
const FORWARDED_RESPONSE_HEADERS: [HeaderName; 2] = [header::CONTENT_TYPE, header::CACHE_CONTROL];

#[derive(Deserialize)]
pub struct RefsQuery {
    service: String,
}

/// [`RefsQuery`] on the one GitLab route, whose RPCs carry no query.
#[derive(Deserialize)]
pub struct GitLabQuery {
    service: Option<String>,
}

/// The repository one proxied request names in its path.
enum Target {
    /// `/git/{repository_id}`: a registered repository.
    Registered(String),
    /// `/git/github/{owner}/{name}`: any github.com repository, registered or
    /// not, which is how the pod's `git-remote-git+https` helper addresses one.
    GitHub { owner: String, name: String },
    /// `/git/gitlab/{host}/{group…}/{name}`: any repository on a GitLab
    /// instance the Workspace may reach, which the pod's `insteadOf` rewrite
    /// of a GitLab URL addresses.
    GitLab { host: String, path: String },
}

impl Target {
    /// How the request log and a stalled body's warning name the repository.
    fn label(&self) -> String {
        match self {
            Self::Registered(repository_id) => repository_id.clone(),
            Self::GitHub { owner, name } => format!("github.com/{owner}/{name}"),
            Self::GitLab { host, path } => format!("{host}/{path}"),
        }
    }
}

/// `GET /git/{repository_id}/info/refs?service=…` — the reference advertisement
/// that opens every clone, fetch and push.
pub async fn info_refs(
    State(proxy): State<Arc<GitProxy>>,
    Path(repository_id): Path<String>,
    Query(query): Query<RefsQuery>,
    headers: HeaderMap,
) -> Response {
    refs(&proxy, Target::Registered(repository_id), &query, &headers).await
}

/// `POST /git/{repository_id}/git-upload-pack` — the fetch negotiation and the
/// packfile that follows it.
pub async fn upload_pack(
    State(proxy): State<Arc<GitProxy>>,
    Path(repository_id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let target = Target::Registered(repository_id);
    service_rpc(&proxy, target, GitService::UploadPack, &headers, body).await
}

/// `POST /git/{repository_id}/git-receive-pack` — the push. The resolved
/// credential must carry `write:repository`, or the host rejects it here.
pub async fn receive_pack(
    State(proxy): State<Arc<GitProxy>>,
    Path(repository_id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let target = Target::Registered(repository_id);
    service_rpc(&proxy, target, GitService::ReceivePack, &headers, body).await
}

/// `GET /git/github/{owner}/{name}/info/refs?service=…`; see [`info_refs`].
pub async fn github_info_refs(
    State(proxy): State<Arc<GitProxy>>,
    Path((owner, name)): Path<(String, String)>,
    Query(query): Query<RefsQuery>,
    headers: HeaderMap,
) -> Response {
    refs(&proxy, Target::GitHub { owner, name }, &query, &headers).await
}

/// `POST /git/github/{owner}/{name}/git-upload-pack`; see [`upload_pack`].
pub async fn github_upload_pack(
    State(proxy): State<Arc<GitProxy>>,
    Path((owner, name)): Path<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let target = Target::GitHub { owner, name };
    service_rpc(&proxy, target, GitService::UploadPack, &headers, body).await
}

/// `POST /git/github/{owner}/{name}/git-receive-pack`; see [`receive_pack`].
pub async fn github_receive_pack(
    State(proxy): State<Arc<GitProxy>>,
    Path((owner, name)): Path<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let target = Target::GitHub { owner, name };
    service_rpc(&proxy, target, GitService::ReceivePack, &headers, body).await
}

/// `/git/gitlab/{host}/{*rest}`, where `rest` is the repository path then
/// `info/refs`, `git-upload-pack` or `git-receive-pack`. One route: a path
/// with nested groups cannot be matched ahead of a fixed suffix.
pub async fn gitlab(
    State(proxy): State<Arc<GitProxy>>,
    Path((host, rest)): Path<(String, String)>,
    method: axum::http::Method,
    Query(query): Query<GitLabQuery>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let target = |path: &str| Target::GitLab {
        host: host.clone(),
        path: path.to_string(),
    };
    if method == axum::http::Method::GET {
        if let (Some(path), Some(service)) = (rest.strip_suffix("/info/refs"), query.service) {
            return refs(&proxy, target(path), &RefsQuery { service }, &headers).await;
        }
    } else if method == axum::http::Method::POST {
        for service in [GitService::UploadPack, GitService::ReceivePack] {
            if let Some(path) = rest.strip_suffix(&format!("/{}", service.name())) {
                return service_rpc(&proxy, target(path), service, &headers, body).await;
            }
        }
    }
    GitProxyError::NotFound.into_response()
}

async fn refs(
    proxy: &GitProxy,
    target: Target,
    query: &RefsQuery,
    headers: &HeaderMap,
) -> Response {
    let Some(service) = service_named(&query.service) else {
        return GitProxyError::NotFound.into_response();
    };
    let upstream = match resolve(proxy, &target, service, headers).await {
        Ok(upstream) => upstream,
        Err(error) => return error.into_response(),
    };
    let url = format!("{}/info/refs?service={}", upstream.base_url, service.name());
    let request = Forwarded {
        label: target.label(),
        service,
        commands: None,
        watch: None,
    };
    forward(
        proxy,
        upstream,
        request,
        reqwest::Method::GET,
        url,
        headers,
        Body::empty(),
    )
    .await
}

async fn service_rpc(
    proxy: &GitProxy,
    target: Target,
    service: GitService,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    let upstream = match resolve(proxy, &target, service, headers).await {
        Ok(upstream) => upstream,
        Err(error) => return error.into_response(),
    };
    let (body, commands, watch) = match service {
        GitService::UploadPack => (body, None, None),
        GitService::ReceivePack => match check_push(proxy, &upstream, headers, body).await {
            Ok((body, commands)) => {
                let moves = commands
                    .as_ref()
                    .map_or_else(Vec::new, |c| c.branch_moves.clone());
                let watch = proxy.push_watch(&upstream, moves);
                (body, commands, watch)
            }
            Err(refused) => return *refused,
        },
    };
    let url = format!("{}/{}", upstream.base_url, service.name());
    let request = Forwarded {
        label: target.label(),
        service,
        commands,
        watch,
    };
    forward(
        proxy,
        upstream,
        request,
        reqwest::Method::POST,
        url,
        headers,
        body,
    )
    .await
}

/// Read a push's command list and run each ref past the policy. The buffered
/// pkt-lines are replayed ahead of the rest of the body when the push goes on,
/// beside the command list, `None` when it could not be read.
async fn check_push(
    proxy: &GitProxy,
    upstream: &Upstream,
    headers: &HeaderMap,
    body: Body,
) -> Result<(Body, Option<Commands>), Box<Response>> {
    let mut stream = body.into_data_stream();
    let mut buf = Vec::new();
    let compressed = headers
        .get(header::CONTENT_ENCODING)
        .is_some_and(|v| v.as_bytes() != b"identity");
    let parsed = if compressed {
        Err("the push is compressed".to_string())
    } else {
        loop {
            match push::parse(&buf) {
                Parsed::Complete { commands, .. } => break Ok(commands),
                Parsed::Invalid(why) => break Err(why),
                Parsed::NeedMore if buf.len() > push::COMMANDS_LIMIT => {
                    break Err("the command list is too long".to_string());
                }
                Parsed::NeedMore => {}
            }
            match stream.next().await {
                Some(Ok(chunk)) => buf.extend_from_slice(&chunk),
                Some(Err(error)) => {
                    warn!(%error, "A push body failed before its command list ended");
                    return Err(Box::new(StatusCode::BAD_REQUEST.into_response()));
                }
                None => break Err("the push ended inside its command list".to_string()),
            }
        }
    };
    let replay = |buf: Vec<u8>, rest| {
        Body::from_stream(futures::stream::once(async move { Ok(Bytes::from(buf)) }).chain(rest))
    };
    let label = format!("{}/{}", upstream.host, upstream.path);
    let request = |path: &'_ str| PolicyRequest {
        proxy: "git",
        sandbox_id: &upstream.sandbox_id,
        workspace_id: &upstream.workspace_id,
        method: "push",
        path: path.to_string(),
    };

    let commands = match parsed {
        Ok(commands) => commands,
        Err(why) => {
            let refusal = Refusal::new(
                "push_unreadable",
                format!("The git proxy could not read this push's refs: {why}"),
            );
            if proxy.policy.refuses(&request(&label), &refusal) {
                return Err(Box::new(
                    (StatusCode::FORBIDDEN, refusal.message).into_response(),
                ));
            }
            return Ok((replay(buf, stream), None));
        }
    };
    let (owner, name) = upstream
        .path
        .split_once('/')
        .unwrap_or((&upstream.path, ""));
    let refused = proxy
        .policy
        .check_push(
            &upstream.workspace_id,
            &github_policy::repo_key(&upstream.host, owner, name),
            &commands.updates,
        )
        .await;
    let mut enforce = false;
    for (ref_name, refusal) in &refused {
        enforce |= proxy
            .policy
            .refuses(&request(&format!("{label}:{ref_name}")), refusal);
    }
    if !enforce {
        proxy
            .claim_pushed_commits(upstream, &commands.pushed_branch_shas)
            .await;
        return Ok((replay(buf, stream), Some(commands)));
    }
    // Read to the end so git sees the report rather than a reset connection.
    while stream.next().await.is_some() {}
    Err(Box::new(match push::report(&commands, &refused) {
        Some(report) => (
            StatusCode::OK,
            [(
                header::CONTENT_TYPE,
                "application/x-git-receive-pack-result",
            )],
            report,
        )
            .into_response(),
        None => (
            StatusCode::FORBIDDEN,
            "The git proxy refused this push's refs",
        )
            .into_response(),
    }))
}

async fn resolve(
    proxy: &GitProxy,
    target: &Target,
    service: GitService,
    headers: &HeaderMap,
) -> Result<Upstream, GitProxyError> {
    let token = basic_password(headers)
        .ok_or_else(|| GitProxyError::Unauthorized("No task token was presented".to_string()))?;
    let push = service == GitService::ReceivePack;
    let upstream = match target {
        Target::Registered(repository_id) => proxy.resolve(repository_id, &token, push).await?,
        Target::GitHub { owner, name } => proxy.resolve_github(owner, name, &token, push).await?,
        Target::GitLab { host, path } => proxy.resolve_gitlab(host, path, &token, push).await?,
    };
    info!(
        repository = %target.label(),
        host = %upstream.host,
        anonymous = upstream.credential.token().is_none(),
        "Proxying a git request"
    );
    Ok(upstream)
}

/// What one proxied request is, beside where it goes.
struct Forwarded {
    /// [`Target::label`].
    label: String,
    service: GitService,
    /// A push's command list, `None` for a fetch or a push the proxy could not
    /// read.
    commands: Option<Commands>,
    /// Started once a successful push's report has passed through whole, or
    /// at once when the host failed the push.
    watch: Option<PushWatch>,
}

async fn forward(
    proxy: &GitProxy,
    upstream: Upstream,
    request: Forwarded,
    method: reqwest::Method,
    url: String,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    let Forwarded {
        label,
        service,
        commands,
        watch,
    } = request;
    let mut request = proxy.http.request(method, &url);
    if let Some(token) = upstream.credential.token() {
        request = request.basic_auth(upstream.username, Some(token));
    }
    let mut request = request.body(reqwest::Body::wrap_stream(
        body.into_data_stream().map_err(std::io::Error::other),
    ));
    for name in FORWARDED_REQUEST_HEADERS {
        if let Some(value) = headers.get(&name) {
            request = request.header(name, value);
        }
    }
    // Git's own capability header. Without it a v2 client's request is answered
    // in v0, and the negotiation it already started fails.
    if let Some(value) = headers.get("git-protocol") {
        request = request.header("git-protocol", value);
    }

    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            warn!(%error, host = %upstream.host, "A proxied git request did not reach its host");
            return GitProxyError::Upstream(format!("{} is unreachable", upstream.host))
                .into_response();
        }
    };

    let status = StatusCode::from_u16(response.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    if status == StatusCode::NOT_FOUND {
        return (status, proxy.refusal(&upstream).await).into_response();
    }
    if service == GitService::ReceivePack && status.is_server_error() {
        return failed_push(response, &upstream.host, commands.as_ref(), watch).await;
    }
    let mut out = Response::builder().status(status);
    for name in FORWARDED_RESPONSE_HEADERS {
        if let Some(value) = response.headers().get(&name)
            && let Ok(value) = HeaderValue::from_bytes(value.as_bytes())
        {
            out = out.header(name, value);
        }
    }
    let mut body = WatchedBody::new(
        Box::pin(response.bytes_stream()),
        service.quiet_limit(),
        label,
        service.name(),
    );
    if let Some(watch) = watch.filter(|_| status.is_success()) {
        body = body.on_complete(move || watch.spawn());
    }
    out.body(Body::from_stream(body)).unwrap_or_else(|error| {
        warn!(%error, "Failed to build a proxied git response");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })
}

/// A push `host` answered with a 5xx, told to git as a rejection of each ref
/// naming the status: git prints no body of a failed RPC, only `HTTP 500`.
///
/// Deliberately not retried here: only the command list is buffered, so the
/// pack cannot be replayed, and the host may have moved a ref before failing.
/// The watch runs for that case, where a client retry finds the ref up to date.
async fn failed_push(
    response: reqwest::Response,
    host: &str,
    commands: Option<&Commands>,
    watch: Option<PushWatch>,
) -> Response {
    let status = response.status();
    let request_id = response
        .headers()
        .get("x-github-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = response.text().await.unwrap_or_default();
    let body: String = body.chars().take(200).collect();
    warn!(%status, host, request_id, body, "The host failed a proxied push");
    if let Some(watch) = watch {
        watch.spawn();
    }
    let request = request_id.map_or_else(String::new, |id| format!(" (request {id})"));
    let reason = format!(
        "{host} failed this push with HTTP {}{request}, so it may not have \
        applied; retrying the push is safe",
        status.as_u16()
    );
    let Some(report) = commands.and_then(|commands| push::failed(commands, &reason)) else {
        let status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        return (status, reason).into_response();
    };
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/x-git-receive-pack-result",
        )],
        report,
    )
        .into_response()
}

/// The password half of an HTTP Basic header, which is where git puts a token.
///
/// The username is ignored: `moatless-git-cred.sh` answers `x-access-token`,
/// and a person retrying a URL by hand types whatever they like.
fn basic_password(headers: &HeaderMap) -> Option<String> {
    use base64::Engine as _;

    let encoded = headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    Some(decoded.split_once(':')?.1.to_string())
}

impl IntoResponse for GitProxyError {
    fn into_response(self) -> Response {
        let status = match self {
            // `WWW-Authenticate` is what makes git ask its credential helper
            // again rather than give up, so a rotated token is picked up on the
            // retry.
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
        };
        let mut response = (status, self.to_string()).into_response();
        if status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Basic realm=\"moatless\""),
            );
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    use std::sync::atomic::{AtomicBool, Ordering};

    use axum::routing::{get, post};

    use crate::git_proxy::push_watch::{PushWatchConfig, Timing};
    use crate::git_proxy::service::Credential;

    const OLD: &str = "1111111111111111111111111111111111111111";
    const NEW: &str = "2222222222222222222222222222222222222222";

    /// GitHub failing receive-pack, and the branch read the watch makes.
    async fn failing_github(watched: Arc<AtomicBool>) -> String {
        let app = axum::Router::new()
            .route(
                "/o/r.git/git-receive-pack",
                post(|| async {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        [("x-github-request-id", "17BE:18A5BD")],
                        "Internal Server Error",
                    )
                }),
            )
            .route(
                "/repos/o/r/git/ref/heads/feat/x",
                get(move || {
                    watched.store(true, Ordering::SeqCst);
                    async { axum::Json(serde_json::json!({ "object": { "sha": OLD } })) }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "a test client against a local server; no `Config` timeouts apply"
    )]
    fn watch(base: &str, commands: &Commands) -> PushWatch {
        PushWatch {
            http: reqwest::Client::new(),
            config: PushWatchConfig {
                api_base: format!("{base}/"),
                timing: Timing {
                    settle: Duration::from_millis(1),
                    bounce_gap: Duration::from_millis(1),
                },
            },
            git_url: format!("{base}/o/r.git"),
            owner: "o".to_string(),
            repo: "r".to_string(),
            username: "x-access-token",
            token: "t".to_string(),
            moves: commands.branch_moves.clone(),
            claim: None,
        }
    }

    #[tokio::test]
    async fn a_push_the_host_fails_is_rejected_naming_the_status_and_still_watched() {
        let watched = Arc::new(AtomicBool::new(false));
        let base = failing_github(watched.clone()).await;
        let command = format!("{OLD} {NEW} refs/heads/feat/x\0report-status side-band-64k\n");
        let mut push = format!("{:04x}{command}0000", command.len() + 4).into_bytes();
        push.extend_from_slice(b"PACK....");
        let Parsed::Complete { commands, .. } = push::parse(&push) else {
            panic!("the command list parses");
        };
        let upstream = Upstream {
            base_url: format!("{base}/o/r.git"),
            credential: Credential::Token("t".to_string()),
            username: "x-access-token",
            host: "github.com".to_string(),
            path: "o/r".to_string(),
            run_as_user_id: "user-1".to_string(),
            workspace_id: "ws_1".to_string(),
            sandbox_id: "sandbox-1".to_string(),
            task_id: None,
        };
        let request = Forwarded {
            label: "github.com/o/r".to_string(),
            service: GitService::ReceivePack,
            watch: Some(watch(&base, &commands)),
            commands: Some(commands),
        };

        let response = forward(
            &crate::git_proxy::service::tests::proxy(1),
            upstream,
            request,
            reqwest::Method::POST,
            format!("{base}/o/r.git/git-receive-pack"),
            &HeaderMap::new(),
            Body::from(push),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK, "git reads a report");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains(
                "ng refs/heads/feat/x github.com failed this push with HTTP 500 \
                (request 17BE:18A5BD), so it may not have applied; retrying the push is safe"
            ),
            "{text}"
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while !watched.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the watch read the branch the failed push named");
    }

    #[test]
    fn a_basic_header_yields_its_password_whatever_the_username() {
        let mut headers = HeaderMap::new();
        let encoded = base64::engine::general_purpose::STANDARD.encode("anyone:task-jwt");
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Basic {encoded}")).unwrap(),
        );
        assert_eq!(basic_password(&headers).as_deref(), Some("task-jwt"));
    }

    #[test]
    fn a_missing_or_unparsable_header_yields_no_password() {
        assert_eq!(basic_password(&HeaderMap::new()), None);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer task-jwt"),
        );
        assert_eq!(basic_password(&headers), None);
    }
}
