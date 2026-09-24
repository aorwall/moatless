//! The one pod port every published preview reaches.
//!
//! A published preview's Service sends `{task}--{port}` here, not to the
//! server's own port; the upstream port comes from the request's `Host`, and
//! the server is always dialled over loopback with a loopback `Host`, so it
//! previews without changing how it is started. Untrusted traffic gets its own
//! listener, reaching only a non-reserved port in the pod's shared namespace;
//! see [`starting_page`] for why a refused connection isn't a gateway error.

use std::convert::Infallible;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{
    ACCEPT, CACHE_CONTROL, CONTENT_TYPE, HOST, HeaderName, HeaderValue, LOCATION, ORIGIN, REFERER,
    RETRY_AFTER,
};
use hyper::service::service_fn;
use hyper::upgrade::OnUpgrade;
use hyper::{HeaderMap, Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use moatless_types::sandbox::is_reserved_sandbox_port;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// The address the upstream server is dialled on, and the `Host` it is told.
const UPSTREAM_ADDR: &str = "127.0.0.1";

const X_FORWARDED_HOST: &str = "x-forwarded-host";
const X_FORWARDED_PROTO: &str = "x-forwarded-proto";

/// Marks [`starting_page`]'s own response, so the page's poll can tell it apart
/// from a `503` the previewed server itself returns once it is up.
const X_MOATLESS_PREVIEW: &str = "x-moatless-preview";
const STARTING: &str = "starting";

/// How long the accept loop yields after a failed `accept`.
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(50);

/// How long the upstream has to return a response head. Generous: a dev server
/// compiling on the first request routinely takes tens of seconds, and the cost
/// of cutting that off is a preview that looks broken.
const UPSTREAM_HEAD_TIMEOUT: Duration = Duration::from_mins(1);

/// Serve preview traffic on `listener` until `cancel` fires.
///
/// Cancellation stops the accept loop; already-accepted connections run in
/// detached tasks and end with the process — a preview is not worth holding a
/// shutdown open for a hung WebSocket.
pub async fn serve(listener: TcpListener, grpc_port: u16, cancel: CancellationToken) {
    let listen_port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(error) => {
            warn!(%error, "Preview proxy could not read its own address");
            return;
        }
    };
    info!(port = listen_port, "Preview proxy started");

    loop {
        let stream = tokio::select! {
            () = cancel.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                // `accept` also fails for reasons that do not clear on their
                // own — `EMFILE`/`ENFILE` when the fd table is full — so the
                // retry yields first. Without it the loop spins a core the
                // agent's turn needs and floods the log used to diagnose it.
                Err(error) => {
                    warn!(%error, "Preview proxy could not accept a connection");
                    tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
                    continue;
                }
            },
        };

        tokio::spawn(async move {
            let service = service_fn(move |request| forward(request, listen_port, grpc_port));
            if let Err(error) = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades()
                .await
            {
                debug!(%error, "Preview proxy connection ended");
            }
        });
    }

    info!("Preview proxy stopped");
}

/// Hand one request to the server the `Host` names.
async fn forward(
    mut request: Request<Incoming>,
    listen_port: u16,
    grpc_port: u16,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, Infallible> {
    let Some(external_host) = request
        .headers()
        .get(HOST)
        .and_then(|host| host.to_str().ok())
        .map(str::to_owned)
    else {
        return Ok(refuse(
            StatusCode::BAD_REQUEST,
            "Preview requests need a Host",
        ));
    };

    let Some(port) = upstream_port(&external_host) else {
        return Ok(refuse(
            StatusCode::BAD_REQUEST,
            "Host does not name a preview port",
        ));
    };

    // `listen_port` is refused separately from the reserved set because it is
    // the one collision the reserved set cannot state: a request naming this
    // proxy's own port would have it dial itself, once per hop, until the
    // connection budget ran out.
    if port == listen_port || is_reserved_sandbox_port(port, grpc_port) {
        debug!(port, "Preview proxy refused a reserved port");
        return Ok(refuse(StatusCode::FORBIDDEN, "Port is not previewable"));
    }

    let upstream_host = format!("{UPSTREAM_ADDR}:{port}");
    let wants_html = is_document_request(request.method(), request.headers());
    // Taken before the request is forwarded: hyper resolves this only if the
    // response it sends back is a 101, so an ordinary request pays nothing.
    let downstream_upgrade = hyper::upgrade::on(&mut request);
    rewrite_request(request.headers_mut(), &upstream_host);

    let stream = match TcpStream::connect((UPSTREAM_ADDR, port)).await {
        Ok(stream) => stream,
        Err(error) => {
            debug!(port, %error, "Preview proxy could not reach the server");
            // Only a refused connection is replaced, and only for a reader: an
            // asset or an `XHR` given HTML would parse it as its own data.
            if wants_html {
                return Ok(starting_page(port));
            }
            return Ok(refuse(
                StatusCode::BAD_GATEWAY,
                "Nothing is listening on that port",
            ));
        }
    };

    let (mut sender, connection) =
        match hyper::client::conn::http1::handshake(TokioIo::new(stream)).await {
            Ok(pair) => pair,
            Err(error) => {
                debug!(port, %error, "Preview proxy handshake failed");
                return Ok(refuse(StatusCode::BAD_GATEWAY, "Upstream handshake failed"));
            }
        };
    tokio::spawn(async move {
        if let Err(error) = connection.with_upgrades().await {
            debug!(%error, "Preview proxy upstream connection ended");
        }
    });

    // Bounded on the response *head* only. A server that accepts the connection
    // and then never answers would otherwise hold this task, its socket and the
    // upstream connection until the process exits. Streaming bodies — SSE, a
    // long download — send their head immediately and are unaffected, and the
    // upgrade path leaves the bridged connection unbounded on purpose.
    let mut response =
        match tokio::time::timeout(UPSTREAM_HEAD_TIMEOUT, sender.send_request(request)).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                debug!(port, %error, "Preview proxy request failed");
                return Ok(refuse(StatusCode::BAD_GATEWAY, "Upstream request failed"));
            }
            Err(_) => {
                debug!(
                    port,
                    "Preview proxy timed out waiting for the server to respond"
                );
                return Ok(refuse(
                    StatusCode::GATEWAY_TIMEOUT,
                    "The server did not respond",
                ));
            }
        };

    if response.status() == StatusCode::SWITCHING_PROTOCOLS {
        let upstream_upgrade = hyper::upgrade::on(&mut response);
        tokio::spawn(bridge(downstream_upgrade, upstream_upgrade));
    } else {
        rewrite_response(response.headers_mut(), &upstream_host, &external_host);
    }

    Ok(response.map(BodyExt::boxed))
}

/// The port `{task}--{port}.{domain}` names, or `None` when the host isn't
/// ours.
///
/// Only the first label is read; `--` is `edge::publish_task_port`'s separator,
/// and the last one wins since a task id contains single dashes.
fn upstream_port(host: &str) -> Option<u16> {
    let label = host.split('.').next()?.split(':').next()?;
    let (_, port) = label.rsplit_once("--")?;
    port.parse().ok().filter(|port| *port != 0)
}

/// Whether this request is a browser asking for a page to render.
///
/// `Accept` is the portable signal: a stylesheet asks for `text/css,*/*`, an
/// image for `image/…`, `fetch`/`curl` for `*/*` — none name `text/html`.
fn is_document_request(method: &Method, headers: &HeaderMap) -> bool {
    method == Method::GET
        && headers
            .get(ACCEPT)
            .and_then(|accept| accept.to_str().ok())
            .is_some_and(|accept| accept.contains("text/html"))
}

/// The `Accept` the page's own poll sends, rendered into it as
/// `{{POLL_ACCEPT}}`.
///
/// Must match a navigation's `Accept`, or it reads `502` and reloads every
/// second: `fetch`'s `*/*` default is what [`is_document_request`] rejects.
const POLL_ACCEPT: &str = "text/html";

/// The page a reader gets while the server on `port` is still starting.
///
/// Status must be one no client caches: `503` with `Retry-After` and the
/// [`X_MOATLESS_PREVIEW`] marker the poll reads, since it polls this same URL
/// and loads the app once the server answers.
fn starting_page(port: u16) -> Response<BoxBody<Bytes, hyper::Error>> {
    let html = include_str!("preview_proxy/starting.html")
        .replace("{{PORT}}", &port.to_string())
        .replace("{{POLL_ACCEPT}}", POLL_ACCEPT);
    let body = Full::new(Bytes::from(html))
        .map_err(|never: Infallible| match never {})
        .boxed();
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .header(CACHE_CONTROL, "no-store")
        .header(RETRY_AFTER, "1")
        .header(HeaderName::from_static(X_MOATLESS_PREVIEW), STARTING)
        .body(body)
        .unwrap_or_else(|_| Response::new(empty_body()))
}

/// Present the request as one made to the server on loopback.
fn rewrite_request(headers: &mut HeaderMap, upstream_host: &str) {
    let forwarded_host = headers.get(HOST).cloned();

    if let Ok(value) = HeaderValue::from_str(upstream_host) {
        headers.insert(HOST, value);
    }
    // Only when already present: adding either would turn a same-origin request
    // into a cross-origin one, which is a check some servers apply.
    if headers.contains_key(ORIGIN) {
        if let Ok(value) = HeaderValue::from_str(&format!("http://{upstream_host}")) {
            headers.insert(ORIGIN, value);
        }
    }
    if let Some(referer) = headers
        .get(REFERER)
        .and_then(|referer| referer.to_str().ok())
        .map(|referer| replace_origin(referer, &format!("http://{upstream_host}")))
        .and_then(|referer| HeaderValue::from_str(&referer).ok())
    {
        headers.insert(REFERER, referer);
    }

    // What the server should build absolute URLs from, for the frameworks that
    // read these rather than `Host`.
    if let Some(host) = forwarded_host {
        headers.insert(HeaderName::from_static(X_FORWARDED_HOST), host);
    }
    headers.insert(
        HeaderName::from_static(X_FORWARDED_PROTO),
        HeaderValue::from_static("https"),
    );
}

/// Undo the loopback rewrite on the way back.
///
/// A server that redirects builds the target from the `Host` it was given, so
/// without this a browser follows the redirect to its own machine.
fn rewrite_response(headers: &mut HeaderMap, upstream_host: &str, external_host: &str) {
    let Some(location) = headers
        .get(LOCATION)
        .and_then(|location| location.to_str().ok())
    else {
        return;
    };
    let upstream_origin = format!("http://{upstream_host}");
    if !location.starts_with(&upstream_origin) {
        return;
    }
    let external_origin = format!("https://{external_host}");
    if let Ok(value) = HeaderValue::from_str(&replace_origin(location, &external_origin)) {
        headers.insert(LOCATION, value);
    }
}

/// `url` with its scheme and authority replaced by `origin`.
fn replace_origin(url: &str, origin: &str) -> String {
    let Some(rest) = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
    else {
        return url.to_string();
    };
    // The authority ends at the first `/`, `?` or `#`. Splitting on `/` alone
    // reads a slash inside a query as the path boundary, so
    // `http://127.0.0.1:3100?next=/admin` rewrites to `{origin}/admin` — a
    // different URL — and a query with no slash in it is dropped entirely.
    let path = rest.find(['/', '?', '#']).map_or("", |end| &rest[end..]);
    format!("{origin}{path}")
}

/// Copy bytes both ways once both halves of an upgraded connection are ready.
async fn bridge(downstream: OnUpgrade, upstream: OnUpgrade) {
    let (downstream, upstream) = match tokio::try_join!(downstream, upstream) {
        Ok(both) => both,
        Err(error) => {
            debug!(%error, "Preview proxy could not complete an upgrade");
            return;
        }
    };
    let mut downstream = TokioIo::new(downstream);
    let mut upstream = TokioIo::new(upstream);
    if let Err(error) = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await {
        debug!(%error, "Preview proxy upgraded connection ended");
    }
}

/// An answer the proxy produced itself, in the body type a forwarded response
/// uses.
fn refuse(status: StatusCode, message: &'static str) -> Response<BoxBody<Bytes, hyper::Error>> {
    let body = Full::new(Bytes::from_static(message.as_bytes()))
        .map_err(|never: Infallible| match never {})
        .boxed();
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(body)
        .unwrap_or_else(|_| Response::new(empty_body()))
}

fn empty_body() -> BoxBody<Bytes, hyper::Error> {
    Full::new(Bytes::new())
        .map_err(|never: Infallible| match never {})
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_types::sandbox::{SANDBOX_HTTP_PORT, SANDBOX_PREVIEW_PROXY_PORT};
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const TASK: &str = "1213ee6f-2b74-47f4-af99-b2bf1ed587df";

    fn preview_host(port: u16) -> String {
        format!("{TASK}--{port}.preview.example.com")
    }

    #[test]
    fn reads_the_port_out_of_a_preview_host() {
        assert_eq!(upstream_port(&preview_host(3100)), Some(3100));
    }

    /// A task id contains single dashes, so only the last `--` separates it
    /// from the port.
    #[test]
    fn a_dashed_task_id_does_not_shift_the_port() {
        assert_eq!(upstream_port("a-b-c--8080.preview.example.com"), Some(8080));
    }

    #[test]
    fn ignores_a_port_on_the_host_itself() {
        assert_eq!(upstream_port(&format!("{TASK}--3100:443")), Some(3100));
    }

    #[test]
    fn refuses_a_host_that_names_no_port() {
        assert_eq!(upstream_port("preview.example.com"), None);
        assert_eq!(upstream_port(&format!("{TASK}--nope.example.com")), None);
        assert_eq!(upstream_port(&format!("{TASK}--0.example.com")), None);
        assert_eq!(upstream_port(&format!("{TASK}--99999.example.com")), None);
    }

    #[test]
    fn the_proxy_port_is_reserved_by_the_shared_rule() {
        assert!(is_reserved_sandbox_port(SANDBOX_PREVIEW_PROXY_PORT, 50052));
    }

    #[test]
    fn a_referer_is_moved_onto_the_upstream_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_static("task--3100.example.com"));
        headers.insert(
            REFERER,
            HeaderValue::from_static("https://task--3100.example.com/a/b?c=d"),
        );

        rewrite_request(&mut headers, "127.0.0.1:3100");

        assert_eq!(headers[HOST], "127.0.0.1:3100");
        assert_eq!(headers[REFERER], "http://127.0.0.1:3100/a/b?c=d");
        assert_eq!(headers[X_FORWARDED_HOST], "task--3100.example.com");
        assert_eq!(headers[X_FORWARDED_PROTO], "https");
    }

    /// Adding one would make a same-origin request look cross-origin.
    #[test]
    fn an_absent_origin_stays_absent() {
        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_static("task--3100.example.com"));

        rewrite_request(&mut headers, "127.0.0.1:3100");

        assert!(!headers.contains_key(ORIGIN));
    }

    #[test]
    fn a_redirect_to_the_upstream_comes_back_pointing_at_the_preview() {
        let mut headers = HeaderMap::new();
        headers.insert(
            LOCATION,
            HeaderValue::from_static("http://127.0.0.1:3100/index.html"),
        );

        rewrite_response(&mut headers, "127.0.0.1:3100", "task--3100.example.com");

        assert_eq!(
            headers[LOCATION],
            "https://task--3100.example.com/index.html"
        );
    }

    #[test]
    fn a_redirect_somewhere_else_is_left_alone() {
        let mut headers = HeaderMap::new();
        headers.insert(
            LOCATION,
            HeaderValue::from_static("https://accounts.example.com/login"),
        );

        rewrite_response(&mut headers, "127.0.0.1:3100", "task--3100.example.com");

        assert_eq!(headers[LOCATION], "https://accounts.example.com/login");
    }

    /// A login bounce redirecting to `?next=…` is an ordinary shape, and a
    /// slash inside that query is not the start of the path.
    #[test]
    fn a_query_is_carried_over_whole_and_never_read_as_a_path() {
        let origin = "https://task--3100.example.com";

        assert_eq!(
            replace_origin("http://127.0.0.1:3100?next=/admin", origin),
            "https://task--3100.example.com?next=/admin"
        );
        assert_eq!(
            replace_origin("http://127.0.0.1:3100?next=1", origin),
            "https://task--3100.example.com?next=1"
        );
        assert_eq!(
            replace_origin("http://127.0.0.1:3100/login?next=/admin", origin),
            "https://task--3100.example.com/login?next=/admin"
        );
    }

    #[test]
    fn a_fragment_ends_the_authority_too() {
        assert_eq!(
            replace_origin("http://127.0.0.1:3100#section", "https://preview.example"),
            "https://preview.example#section"
        );
    }

    /// Start the proxy on an ephemeral port and return it.
    async fn start_proxy() -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("proxy binds");
        let port = listener.local_addr().expect("proxy address").port();
        tokio::spawn(serve(listener, 50052, CancellationToken::new()));
        port
    }

    /// A server bound to loopback only, the case the proxy exists for and the
    /// pod's own address could not reach.
    ///
    /// Answers with the `Host` it was given, so a test can assert what reached
    /// it, not just what came back.
    async fn start_loopback_server(response: &'static str) -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("server binds");
        let port = listener.local_addr().expect("server address").port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buffer = [0u8; 2048];
                    let read = stream.read(&mut buffer).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                    // hyper writes header names lower-cased on the wire.
                    let host = request
                        .lines()
                        .find(|line| line.to_ascii_lowercase().starts_with("host:"))
                        .and_then(|line| line.split_once(':'))
                        .map_or_else(String::new, |(_, value)| value.trim().to_string());
                    let body = format!("host={host}");
                    let head = response.replace("{len}", &body.len().to_string());
                    let _ = stream.write_all(head.as_bytes()).await;
                    if head.starts_with("HTTP/1.1 200") {
                        let _ = stream.write_all(body.as_bytes()).await;
                    }
                    // An upgraded connection echoes whatever follows.
                    if head.starts_with("HTTP/1.1 101") {
                        let mut echo = [0u8; 64];
                        if let Ok(n) = stream.read(&mut echo).await {
                            let _ = stream.write_all(&echo[..n]).await;
                        }
                    }
                    let _ = stream.flush().await;
                });
            }
        });
        port
    }

    /// Send one raw request to the proxy and read what comes back.
    async fn request(proxy_port: u16, host: &str, extra: &str) -> (String, TcpStream) {
        let addr = SocketAddr::from(([127, 0, 0, 1], proxy_port));
        let mut stream = TcpStream::connect(addr).await.expect("proxy accepts");
        let request = format!("GET / HTTP/1.1\r\nHost: {host}\r\n{extra}\r\n");
        stream
            .write_all(request.as_bytes())
            .await
            .expect("request written");
        let mut buffer = [0u8; 4096];
        let read = stream.read(&mut buffer).await.expect("response read");
        (String::from_utf8_lossy(&buffer[..read]).to_string(), stream)
    }

    const OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n";

    /// The whole point: a server that binds `127.0.0.1` is reachable, and it
    /// sees a loopback `Host` rather than the preview hostname.
    #[tokio::test]
    async fn a_loopback_server_is_reached_and_sees_a_loopback_host() {
        let upstream = start_loopback_server(OK).await;
        let proxy = start_proxy().await;

        let (mut response, mut stream) = request(proxy, &preview_host(upstream), "").await;
        stream
            .read_to_string(&mut response)
            .await
            .expect("response body read");

        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(
            response.ends_with(&format!("host=127.0.0.1:{upstream}")),
            "{response}"
        );
    }

    #[tokio::test]
    async fn the_control_port_is_refused_without_being_dialled() {
        let proxy = start_proxy().await;

        let (response, _stream) = request(proxy, &preview_host(SANDBOX_HTTP_PORT), "").await;

        assert!(response.starts_with("HTTP/1.1 403"), "{response}");
    }

    /// Forwarding to itself would recurse until the connection budget ran out.
    #[tokio::test]
    async fn the_proxys_own_port_is_refused() {
        let proxy = start_proxy().await;

        let (response, _stream) = request(proxy, &preview_host(proxy), "").await;

        assert!(response.starts_with("HTTP/1.1 403"), "{response}");
    }

    /// Bind and drop, so the port is known-free rather than guessed.
    async fn free_port() -> u16 {
        TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("binds")
            .local_addr()
            .expect("address")
            .port()
    }

    const NAVIGATION: &str = "Accept: text/html,application/xhtml+xml\r\n";

    /// Anything that is not a reader gets the error, so the previewed app never
    /// parses the page as its own data.
    #[tokio::test]
    async fn a_port_with_no_listener_answers_502() {
        let proxy = start_proxy().await;
        let free = free_port().await;

        let (response, _stream) = request(proxy, &preview_host(free), "").await;

        assert!(response.starts_with("HTTP/1.1 502"), "{response}");
        assert!(response.contains("content-type: text/plain"), "{response}");
    }

    #[tokio::test]
    async fn a_reader_waiting_on_a_starting_server_gets_the_page() {
        let proxy = start_proxy().await;
        let free = free_port().await;

        let (response, _stream) = request(proxy, &preview_host(free), NAVIGATION).await;

        assert!(response.starts_with("HTTP/1.1 503"), "{response}");
        assert!(response.contains("content-type: text/html"), "{response}");
        assert!(response.contains("retry-after: 1"), "{response}");
        assert!(
            response.contains(&format!("{X_MOATLESS_PREVIEW}: {STARTING}")),
            "{response}"
        );
    }

    /// The reader is told which port is still closed, and the poll that loads
    /// the app once it opens needs the marker's own name.
    #[tokio::test]
    async fn the_starting_page_names_the_port_and_the_marker_its_poll_reads() {
        let body = starting_page(3001)
            .into_body()
            .collect()
            .await
            .expect("page body")
            .to_bytes();
        let html = String::from_utf8(body.to_vec()).expect("page is utf-8");

        assert!(html.contains("3001"), "{html}");
        assert!(html.contains(X_MOATLESS_PREVIEW), "{html}");
        assert!(html.contains(&format!("Accept: '{POLL_ACCEPT}'")), "{html}");
        // Every placeholder is rendered, whatever it is named.
        assert!(!html.contains("{{"), "{html}");
    }

    /// The page replaces only a *failed* connect, so a server that is up is
    /// still forwarded to whatever the reader asked for.
    #[tokio::test]
    async fn a_reader_reaching_a_running_server_is_forwarded() {
        let upstream = start_loopback_server(OK).await;
        let proxy = start_proxy().await;

        let (response, _stream) = request(proxy, &preview_host(upstream), NAVIGATION).await;

        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    }

    #[test]
    fn only_a_get_asking_for_html_is_a_document_request() {
        let accepting = |value: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert(ACCEPT, HeaderValue::from_static(value));
            headers
        };

        assert!(is_document_request(
            &Method::GET,
            &accepting("text/html,application/xhtml+xml,*/*;q=0.8")
        ));
        // The page's poll reaches the same arm as a navigation, or it reads the
        // plain `502` and reloads into itself.
        assert!(is_document_request(&Method::GET, &accepting(POLL_ACCEPT)));
        // What `fetch` sends when it is given no `Accept` of its own.
        assert!(!is_document_request(&Method::GET, &accepting("*/*")));
        assert!(!is_document_request(
            &Method::GET,
            &accepting("text/css,*/*;q=0.1")
        ));
        assert!(!is_document_request(
            &Method::GET,
            &accepting("image/avif,image/webp,*/*")
        ));
        assert!(!is_document_request(&Method::GET, &HeaderMap::new()));
        assert!(!is_document_request(&Method::POST, &accepting("text/html")));
    }

    const REDIRECT: &str = "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    #[tokio::test]
    async fn a_redirect_comes_back_addressed_to_the_preview_host() {
        // The upstream's own port is not known until it binds, so the fixture
        // is patched once the port is.
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("server binds");
        let upstream = listener.local_addr().expect("address").port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let head = REDIRECT.replace("{port}", &upstream.to_string());
                let mut buffer = [0u8; 1024];
                let _ = stream.read(&mut buffer).await;
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.flush().await;
            }
        });
        let proxy = start_proxy().await;

        let (response, _stream) = request(proxy, &preview_host(upstream), "").await;

        assert!(
            response.contains(&format!("location: https://{}", preview_host(upstream)))
                || response.contains(&format!("Location: https://{}", preview_host(upstream))),
            "{response}"
        );
    }

    const SWITCHING: &str =
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n";

    /// Dev servers push reloads over a WebSocket, so a proxy that only carried
    /// request/response would break the reload without breaking the page.
    #[tokio::test]
    async fn an_upgraded_connection_carries_bytes_both_ways() {
        let upstream = start_loopback_server(SWITCHING).await;
        let proxy = start_proxy().await;

        let (response, mut stream) = request(
            proxy,
            &preview_host(upstream),
            "Connection: Upgrade\r\nUpgrade: websocket\r\n",
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 101"), "{response}");

        stream.write_all(b"ping").await.expect("sent after upgrade");
        let mut echo = [0u8; 4];
        stream.read_exact(&mut echo).await.expect("echo read");
        assert_eq!(&echo, b"ping");
    }
}
