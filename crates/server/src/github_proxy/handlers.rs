//! The one GitHub proxy route, streamed in both directions.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use futures::{StreamExt, TryStreamExt};
use tracing::{info, warn};

use moatless_core::proxy_rules::NormalPath;
use moatless_workspace_types::repository::GitHostRepo;

use crate::github_policy::{GitHubPolicy, Refusal, Request as PolicyRequest};
use moatless_adapter_github_pr::own_event;

use crate::github_proxy::authorship;
use crate::github_proxy::made;
use crate::github_proxy::service::{GitHubProxy, GitHubProxyError, GitHubUpstream};
use crate::github_proxy::target;
use crate::proxy::caller::SandboxScope;
use crate::proxy::forward::{bearer_token, forwards, respond, stream_back};

/// `ANY /github/{upstream}/{path}` — one `gh` request, forwarded to
/// `upstream` with the sandbox's GitHub token in place of the task token.
pub async fn proxy(
    State(proxy): State<Arc<GitHubProxy>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some((upstream, path_and_query)) = split_upstream(&uri) else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(
                serde_json::json!({ "message": "Only api and uploads are proxied to GitHub" }),
            ),
        )
            .into_response();
    };
    let scope = match proxy.authenticate(bearer_token(&headers)).await {
        Ok(scope) => scope,
        Err(e) => return e.into_response(),
    };
    forward(
        &proxy,
        &scope,
        upstream,
        method,
        &path_and_query,
        &headers,
        body,
    )
    .await
}

/// The upstream `/github/{upstream}/…` names, and the path and query to
/// request from it, or `None` for an upstream this proxy does not forward to.
fn split_upstream(uri: &Uri) -> Option<(GitHubUpstream, String)> {
    let rest = uri.path().strip_prefix("/github/")?;
    let (segment, path) = rest.split_once('/').unwrap_or((rest, ""));
    let upstream = GitHubUpstream::named(segment)?;
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    Some((upstream, format!("/{path}{query}")))
}

/// Send one request upstream as `scope`'s GitHub identity.
pub(super) async fn forward(
    proxy: &GitHubProxy,
    scope: &SandboxScope,
    upstream: GitHubUpstream,
    method: Method,
    path_and_query: &str,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    let graphql = is_graphql(upstream, &method, path_and_query);
    let (requested, body, write) = if graphql {
        let Ok(bytes) = axum::body::to_bytes(body, GRAPHQL_BODY_LIMIT).await else {
            return too_large("The GraphQL request body is too large");
        };
        if let Err(refusal) = proxy.policy.check_graphql(&scope.workspace, &bytes).await {
            if let Some(refused) = refuse(proxy, scope, &method, path_and_query, &refusal) {
                return refused;
            }
        }
        let write = GitHubPolicy::graphql_mutates(&bytes).then(|| Write {
            credits: GitHubPolicy::graphql_runs_only(&bytes, authorship::MUTATIONS),
            opens_pull_request: GitHubPolicy::graphql_runs_only(
                &bytes,
                &[authorship::OPENS_PULL_REQUEST],
            ),
            bytes: bytes.clone(),
        });
        (
            target::from_graphql(&bytes),
            reqwest::Body::from(bytes),
            write,
        )
    } else if !GitHubPolicy::is_write(method.as_str()) {
        (
            target::from_path(path_and_query),
            reqwest::Body::wrap_stream(body.into_data_stream().map_err(std::io::Error::other)),
            None,
        )
    } else {
        // Over the limit it goes on unread, as the run-as identity.
        let (body, write) = match buffer(body, REVIEW_BODY_LIMIT).await {
            Buffered::Whole(bytes) => (
                Body::from(bytes.clone()),
                Some(Write {
                    credits: authorship::rest_authors(&method, path_and_query),
                    opens_pull_request: authorship::rest_opens_pull_request(
                        &method,
                        path_and_query,
                    ),
                    bytes,
                }),
            ),
            Buffered::Over(body) => (body, None),
            Buffered::Failed => return StatusCode::BAD_REQUEST.into_response(),
        };
        match check_rest(proxy, scope, &method, path_and_query, body).await {
            Ok(body) => (target::from_path(path_and_query), body, write),
            Err(refused) => return *refused,
        }
    };
    let request = Outbound {
        proxy,
        scope,
        upstream,
        method: &method,
        path_and_query,
        headers,
        plain: write.is_some(),
    };
    let Some(write) = write else {
        let token = match proxy.token(scope, requested.as_ref()).await {
            Ok(token) => token,
            Err(e) => return e.into_response(),
        };
        return match request.send(token, body, false).await {
            Ok(response) => stream_back(response, "github"),
            Err(refused) => *refused,
        };
    };
    match send_write(&request, graphql, requested.as_ref(), &write).await {
        Ok((response, repository)) => {
            record_made(&request, &write, graphql, repository.as_ref(), response).await
        }
        Err(refused) => *refused,
    }
}

/// Above any answer GitHub gives a write; past it nothing is recorded.
const WRITE_ANSWER_LIMIT: usize = 16 * 1024 * 1024;

enum Answer {
    Whole(Bytes),
    /// The bytes read so far, then the rest of the stream, which an error or
    /// [`WRITE_ANSWER_LIMIT`] stopped reading.
    Partial(Body),
}

/// GitHub's answer whole, or handed on as read. Never an error response of
/// its own: the write already landed, so one would invite a duplicating retry.
async fn read_answer(body: Body) -> Answer {
    let mut stream = body.into_data_stream();
    let mut read = Vec::new();
    let mut size = 0;
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(chunk) => {
                size += chunk.len();
                read.push(Ok::<_, axum::Error>(chunk));
                if size > WRITE_ANSWER_LIMIT {
                    break;
                }
            }
            Err(error) => {
                read.push(Err(error));
                break;
            }
        }
    }
    if size <= WRITE_ANSWER_LIMIT && read.iter().all(Result::is_ok) {
        let mut whole = Vec::with_capacity(size);
        for chunk in read.into_iter().flatten() {
            whole.extend_from_slice(&chunk);
        }
        return Answer::Whole(Bytes::from(whole));
    }
    Answer::Partial(Body::from_stream(futures::stream::iter(read).chain(stream)))
}

/// Record what a successful write made as the Sandbox's Task's own, then hand
/// GitHub's answer back unchanged.
///
/// After the answer rather than before: the id or sha exists only in it, so a
/// webhook beating the answer here is delivered.
async fn record_made(
    request: &Outbound<'_>,
    write: &Write,
    graphql: bool,
    repository: Option<&GitHostRepo>,
    response: Response,
) -> Response {
    let (Some(own_events), Some(task_id), Some(repository)) = (
        request.proxy.own_events.as_ref(),
        request.scope.task.as_deref(),
        repository,
    ) else {
        return response;
    };
    if !response.status().is_success() {
        return response;
    }
    let (parts, body) = response.into_parts();
    let bytes = match read_answer(body).await {
        Answer::Whole(bytes) => bytes,
        Answer::Partial(body) => {
            warn!("GitHub's answer to a proxied write was not read whole; recording nothing");
            return Response::from_parts(parts, body);
        }
    };
    let made = if graphql {
        made::from_graphql(&write.bytes, &bytes)
    } else {
        made::from_rest(request.method, request.path_and_query, &write.bytes, &bytes)
    };
    for made in &made {
        own_events
            .record_best_effort(
                task_id,
                own_event::event(repository.owner(), repository.name(), made),
            )
            .await;
    }
    Response::from_parts(parts, Body::from(bytes))
}

/// A write's body, held whole so the App can send it and the run-as identity
/// resend it.
struct Write {
    bytes: Bytes,
    /// It posts text, which credits whom [`GitHubProxy::credited_login`] names.
    credits: bool,
    /// It opens a pull request, whose description also names whom
    /// [`GitHubProxy::pull_request_credits`] does.
    opens_pull_request: bool,
}

/// One request's destination and the client's headers for it.
struct Outbound<'a> {
    proxy: &'a GitHubProxy,
    scope: &'a SandboxScope,
    upstream: GitHubUpstream,
    method: &'a Method,
    path_and_query: &'a str,
    headers: &'a HeaderMap,
    /// Asks for an unencoded answer, which [`record_made`] reads.
    plain: bool,
}

impl Outbound<'_> {
    /// Send it once with `token`; `rewritten` drops the client's
    /// `Content-Length`, which no longer describes `body`.
    async fn send(
        &self,
        token: String,
        body: reqwest::Body,
        rewritten: bool,
    ) -> Result<reqwest::Response, Box<Response>> {
        let url = self.proxy.upstream_url(self.upstream, self.path_and_query);
        let mut request = self
            .proxy
            .http
            .request(self.method.clone(), &url)
            .body(body);
        for (name, value) in self.headers {
            if forwards(name)
                && name != header::AUTHORIZATION
                && !(rewritten && name == header::CONTENT_LENGTH)
                && !(self.plain && name == header::ACCEPT_ENCODING)
            {
                request = request.header(name, value);
            }
        }
        let response = request.bearer_auth(token).send().await.map_err(|error| {
            warn!(%error, upstream = ?self.upstream, "A proxied GitHub request did not reach GitHub");
            Box::new(
                GitHubProxyError::Upstream("GitHub is unreachable".to_string()).into_response(),
            )
        })?;
        info!(
            sandbox_id = %self.scope.sandbox,
            method = %self.method,
            upstream = ?self.upstream,
            path = self.path_and_query.split('?').next().unwrap_or_default(),
            status = response.status().as_u16(),
            "Proxied a GitHub request"
        );
        Ok(response)
    }

    /// `write`'s body, credited when it posts text and someone is named.
    async fn body(&self, write: &Write, graphql: bool, by_app: bool) -> (reqwest::Body, bool) {
        let login = if write.credits {
            self.proxy.credited_login(self.scope, by_app).await
        } else {
            None
        };
        if write.opens_pull_request {
            let credits = self.proxy.pull_request_credits(self.scope).await;
            let credited = authorship::credit_opened_pull_request(
                &write.bytes,
                graphql,
                &credits,
                login.as_deref(),
            );
            return (reqwest::Body::from(credited), true);
        }
        match login {
            Some(login) => (
                reqwest::Body::from(authorship::credit(&write.bytes, graphql, &login)),
                true,
            ),
            None => (reqwest::Body::from(write.bytes.clone()), false),
        }
    }
}

/// Send a write as the App, per [`GitHubProxy::app_write`], and resend it as
/// [`GitHubProxy::token`] picks when there is no App token or GitHub refuses
/// the App, as for a permission its installation lacks. A pull request the
/// App would open goes per [`open_as_driver`] first.
async fn send_write(
    request: &Outbound<'_>,
    graphql: bool,
    requested: Option<&GitHostRepo>,
    write: &Write,
) -> Result<(Response, Option<GitHostRepo>), Box<Response>> {
    let (proxy, scope) = (request.proxy, request.scope);
    let repository = match (requested, graphql) {
        (None, true) => match authorship::graphql_subject(&write.bytes) {
            Some(id) => proxy.repository_of_node(scope, &id).await,
            None => None,
        },
        _ => requested.cloned(),
    };
    // A REST write naming no repository, such as a gist, has none to scope an
    // App token to.
    let app = match (&repository, graphql) {
        (None, false) => None,
        _ => proxy
            .app_write(scope, repository.clone())
            .await
            .map_err(|e| Box::new(e.into_response()))?,
    };
    if let Some(app) = app {
        if write.opens_pull_request {
            if let Some(opened) = open_as_driver(request, graphql, requested, write).await? {
                return Ok((opened, repository));
            }
        }
        let (body, rewritten) = request.body(write, graphql, true).await;
        let response = request.send(app.token, body, rewritten).await?;
        match refused(response, graphql).await {
            Ok(answered) => return Ok((answered, repository)),
            Err((status, message)) => warn!(
                status,
                message,
                path = request.path_and_query.split('?').next().unwrap_or_default(),
                "GitHub refused a write as the App; resending it as the run-as identity"
            ),
        }
    }
    let token = proxy
        .token(scope, requested)
        .await
        .map_err(|e| Box::new(e.into_response()))?;
    let (body, rewritten) = request.body(write, graphql, false).await;
    let response = request.send(token, body, rewritten).await?;
    Ok((stream_back(response, "github"), repository))
}

/// Open a pull request the App would open with the driver's own token, so
/// GitHub shows the driver as its author; `None` when they have no token or
/// GitHub refuses it, and the App opens it instead.
async fn open_as_driver(
    request: &Outbound<'_>,
    graphql: bool,
    requested: Option<&GitHostRepo>,
    write: &Write,
) -> Result<Option<Response>, Box<Response>> {
    let token = match request.proxy.token(request.scope, requested).await {
        Ok(token) => token,
        Err(error) => {
            warn!(%error, "No driver token to open a pull request with; opening it as the App");
            return Ok(None);
        }
    };
    let (body, rewritten) = request.body(write, graphql, false).await;
    let response = request.send(token, body, rewritten).await?;
    match refused(response, graphql).await {
        Ok(answered) => Ok(Some(answered)),
        Err((status, message)) => {
            warn!(
                status,
                message,
                "GitHub refused to open a pull request as the driver; opening it as the App"
            );
            Ok(None)
        }
    }
}

/// GitHub's answer to a write, or `Err` with its status and message when
/// GitHub refused the token and wrote nothing: a REST 403 or 404 that is no
/// rate limit, or a GraphQL `FORBIDDEN` with no mutation's result.
async fn refused(response: reqwest::Response, graphql: bool) -> Result<Response, (u16, String)> {
    let status = response.status();
    let refusable = graphql || matches!(status.as_u16(), 403 | 404);
    if !refusable || rate_limited(response.headers()) {
        return Ok(stream_back(response, "github"));
    }
    let headers = response.headers().clone();
    let Ok(bytes) = response.bytes().await else {
        return Err((status.as_u16(), "unreadable answer".to_string()));
    };
    if !graphql || authorship::graphql_forbidden(&bytes) {
        return Err((status.as_u16(), authorship::refusal_message(&bytes)));
    }
    Ok(respond(status, &headers, Body::from(bytes), "github"))
}

/// Whether GitHub answered with a rate limit, which the run-as identity's
/// token would only spend more of.
fn rate_limited(headers: &reqwest::header::HeaderMap) -> bool {
    headers.contains_key(reqwest::header::RETRY_AFTER)
        || headers
            .get("x-ratelimit-remaining")
            .is_some_and(|remaining| remaining == "0")
}

/// Run a REST write past the policy, buffering its body only when the policy
/// reads it.
async fn check_rest(
    proxy: &GitHubProxy,
    scope: &SandboxScope,
    method: &Method,
    path_and_query: &str,
    body: Body,
) -> Result<reqwest::Body, Box<Response>> {
    let path = match NormalPath::parse(path_and_query) {
        Ok(path) => path,
        Err(e) => {
            let refusal = Refusal::new("bad_path", format!("Refused {path_and_query}: {e}"));
            return match refuse(proxy, scope, method, path_and_query, &refusal) {
                Some(refused) => Err(Box::new(refused)),
                None => Ok(stream(body)),
            };
        }
    };
    let (decision, body) = if GitHubPolicy::rest_needs_body(method.as_str(), &path) {
        let bytes = match buffer(body, REVIEW_BODY_LIMIT).await {
            Buffered::Whole(bytes) => bytes,
            Buffered::Failed => return Err(Box::new(StatusCode::BAD_REQUEST.into_response())),
            Buffered::Over(body) => {
                let body = stream(body);
                let refusal = Refusal::new(
                    "review_unreadable",
                    "The review body is over 1 MiB, too large for the GitHub proxy to read its event",
                );
                return match refuse(proxy, scope, method, path_and_query, &refusal) {
                    Some(refused) => Err(Box::new(refused)),
                    None => Ok(body),
                };
            }
        };
        let decision = proxy
            .policy
            .check_rest(&scope.workspace, method.as_str(), &path, Some(&bytes))
            .await;
        (decision, reqwest::Body::from(bytes))
    } else {
        let decision = proxy
            .policy
            .check_rest(&scope.workspace, method.as_str(), &path, None)
            .await;
        (decision, stream(body))
    };
    match decision {
        Ok(()) => Ok(body),
        Err(refusal) => match refuse(proxy, scope, method, path_and_query, &refusal) {
            Some(refused) => Err(Box::new(refused)),
            None => Ok(body),
        },
    }
}

enum Buffered {
    Whole(Bytes),
    /// Over the limit: what was read, chained ahead of the rest.
    Over(Body),
    /// The client's body failed mid-read.
    Failed,
}

/// Read `body` whole when it fits in `limit` bytes.
async fn buffer(body: Body, limit: usize) -> Buffered {
    let mut stream = body.into_data_stream();
    let mut buf = Vec::new();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return Buffered::Failed;
        };
        buf.extend_from_slice(&chunk);
        if buf.len() > limit {
            let head = futures::stream::once(async move { Ok::<_, axum::Error>(Bytes::from(buf)) });
            return Buffered::Over(Body::from_stream(head.chain(stream)));
        }
    }
    Buffered::Whole(Bytes::from(buf))
}

fn stream(body: Body) -> reqwest::Body {
    reqwest::Body::wrap_stream(body.into_data_stream().map_err(std::io::Error::other))
}

/// The 403 for `refusal` under `enforce`; `None` under `log`, where the
/// request goes on.
fn refuse(
    proxy: &GitHubProxy,
    scope: &SandboxScope,
    method: &Method,
    path_and_query: &str,
    refusal: &Refusal,
) -> Option<Response> {
    let request = PolicyRequest {
        proxy: "github",
        sandbox_id: &scope.sandbox,
        workspace_id: &scope.workspace,
        method: method.as_str(),
        path: path_and_query
            .split('?')
            .next()
            .unwrap_or_default()
            .to_string(),
    };
    proxy.policy.refuses(&request, refusal).then(|| {
        (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({ "message": refusal.message })),
        )
            .into_response()
    })
}

fn too_large(message: &str) -> Response {
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        axum::Json(serde_json::json!({ "message": message })),
    )
        .into_response()
}

/// Bounds a review body buffered to read its `event`.
const REVIEW_BODY_LIMIT: usize = 1024 * 1024;

/// Bounds the GraphQL body buffered to read its operations and the repository
/// its variables name. A mutation carrying a pull request body is the largest
/// `gh` sends.
const GRAPHQL_BODY_LIMIT: usize = 4 * 1024 * 1024;

fn is_graphql(upstream: GitHubUpstream, method: &Method, path_and_query: &str) -> bool {
    upstream == GitHubUpstream::Api
        && method == Method::POST
        && path_and_query.split('?').next() == Some("/graphql")
}

impl IntoResponse for GitHubProxyError {
    /// A GitHub-shaped error body: `gh` prints its `message`.
    fn into_response(self) -> Response {
        let status = match self {
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::NoGitHubIdentity(_) => StatusCode::FORBIDDEN,
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
        };
        (
            status,
            axum::Json(serde_json::json!({ "message": self.to_string() })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests;
