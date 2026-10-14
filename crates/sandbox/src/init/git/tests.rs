use super::*;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::config::test_support::minimal_config;

/// A port nothing listens on. Well below the ephemeral range, so a
/// concurrently-running test cannot be handed it and make the connect
/// succeed.
const DEAD_PORT: u16 = 1;

fn checkout(mount: &str, provider: &str, is_primary: bool) -> RepoCheckout {
    RepoCheckout {
        uses_ssh: false,
        clone_url: format!("https://{provider}.example/acme/{mount}.git"),
        repository_id: String::new(),
        branch: None,
        provider: provider.to_string(),
        mount_name: mount.to_string(),
        is_primary,
    }
}

fn run_git_sync(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git command runs");
    assert!(status.success(), "git {args:?} failed");
}

fn init_local_source_repo(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    run_git_sync(path, &["init", "--initial-branch", "main"]);
    std::fs::write(path.join("README.md"), "secondary\n").unwrap();
    run_git_sync(path, &["add", "README.md"]);
    run_git_sync(
        path,
        &[
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test User",
            "commit",
            "-m",
            "init",
        ],
    );
}

/// A repo whose directory already holds a `.git` is left alone, so a
/// restart onto a persistent volume neither re-clones nor reports a
/// reclone.
#[tokio::test]
async fn already_cloned_repos_are_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().display().to_string();
    let mut config = minimal_config();
    config.workspace_dir = workspace.clone();
    config.repos = vec![checkout("api", "github", true)];
    config.repo_dir = format!("{workspace}/api");
    std::fs::create_dir_all(format!("{workspace}/api/.git")).unwrap();

    let summary = clone_repositories(&config, &GitCredentials::default())
        .await
        .unwrap();
    assert!(summary.cloned.is_empty());
    assert!(!summary.primary_recloned);
}

/// A restore can carry an older workspace shape: the primary repo present
/// but a later-added sibling absent.
///
/// The clone pass after restore must clone only the missing sibling, and
/// not count the primary as recloned.
#[tokio::test]
async fn restored_primary_does_not_prevent_new_sibling_clone() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    let source = tmp.path().join("source-secondary");
    init_local_source_repo(&source);

    let workspace_str = workspace.display().to_string();
    let primary = workspace.join("primary");
    std::fs::create_dir_all(primary.join(".git")).unwrap();

    let mut config = minimal_config();
    config.workspace_dir = workspace_str.clone();
    config.repo_dir = primary.display().to_string();
    config.git_provider = "local".to_string();
    config.repos = vec![
        RepoCheckout {
            uses_ssh: false,
            clone_url: "https://example.test/acme/primary.git".to_string(),
            repository_id: String::new(),
            branch: None,
            provider: "local".to_string(),
            mount_name: "primary".to_string(),
            is_primary: true,
        },
        RepoCheckout {
            uses_ssh: false,
            clone_url: source.display().to_string(),
            repository_id: String::new(),
            branch: None,
            provider: "local".to_string(),
            mount_name: "secondary".to_string(),
            is_primary: false,
        },
    ];

    let summary = clone_repositories(&config, &GitCredentials::default())
        .await
        .unwrap();

    assert_eq!(summary.cloned, vec!["secondary"]);
    assert!(!summary.primary_recloned);
    assert!(workspace.join("secondary").join(".git").is_dir());
    assert!(workspace.join("secondary").join("README.md").is_file());
}

#[test]
fn a_clone_is_blobless_rather_than_shallow() {
    assert_eq!(
        git_clone_args("https://example.test/acme/api.git", "/w/api", Some("dev")),
        vec![
            "clone",
            "--filter=blob:none",
            "--branch",
            "dev",
            "--",
            "https://example.test/acme/api.git",
            "/w/api",
        ],
    );
    // `--depth` back at any site restores the single-branch refspec that
    // breaks `--force-with-lease` and `origin/main...HEAD`.
    assert!(!git_clone_args("u", "d", None).contains(&"--depth"));
}

/// The three things a `--depth 1` clone took away: a branch other than the
/// clone branch, a lease-eligible tracking ref, and a merge base.
#[tokio::test]
async fn a_cloned_repo_carries_every_branch_and_a_merge_base() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    let source = tmp.path().join("source");
    init_local_source_repo(&source);
    run_git_sync(&source, &["branch", "feature"]);

    let workspace_str = workspace.display().to_string();
    let mut config = minimal_config();
    config.workspace_dir = workspace_str.clone();
    config.repo_dir = workspace.join("api").display().to_string();
    config.git_provider = "local".to_string();
    config.repos = vec![RepoCheckout {
        uses_ssh: false,
        clone_url: source.display().to_string(),
        repository_id: String::new(),
        branch: Some("main".to_string()),
        provider: "local".to_string(),
        mount_name: "api".to_string(),
        is_primary: true,
    }];

    clone_repositories(&config, &GitCredentials::default())
        .await
        .unwrap();

    let clone = workspace.join("api");
    let read = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&clone)
            .args(args)
            .output()
            .expect("git command runs");
        assert!(out.status.success(), "git {args:?} failed");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    assert_eq!(read(&["rev-parse", "--is-shallow-repository"]), "false");
    assert_eq!(
        read(&["config", "--get-all", "remote.origin.fetch"]),
        "+refs/heads/*:refs/remotes/origin/*"
    );
    // A branch the clone did not check out still has a tracking ref, which
    // is what a bare `--force-with-lease` leases against.
    read(&["rev-parse", "--verify", "refs/remotes/origin/feature"]);
    read(&["merge-base", "origin/main", "HEAD"]);
}

/// Each checkout authenticates with its own provider's token; the pod's
/// `GIT_TOKEN` backs up only a checkout on the pod's own provider.
///
/// Injecting it into another host's clone URL would hand the credential to
/// a third party.
#[test]
fn each_checkout_uses_its_own_providers_token() {
    let credentials = credentials(&[("GITLAB_TOKEN", "glpat"), (GIT_TOKEN, "pod-github-token")]);
    assert_eq!(
        credentials.provider_token("gitlab", "github"),
        Some("glpat".to_string())
    );
    // No `GITHUB_TOKEN` was delivered, so GitHub uses the pod's.
    assert_eq!(
        credentials.provider_token("github", "github"),
        Some("pod-github-token".to_string())
    );
    // A foreign provider with no token of its own gets nothing.
    assert_eq!(credentials.provider_token("forgejo", "github"), None);
}

/// A proxied checkout's URL points at the backend, so no provider token may
/// be offered to it — not even the pod's own.
#[test]
fn a_proxied_checkout_is_given_no_provider_token() {
    let credentials = credentials(&[("GITHUB_TOKEN", "pod-token"), (GIT_TOKEN, "pod-token")]);
    let mut config = crate::config::test_support::minimal_config();
    config.backend_api_url = Some("http://backend:8001".to_string());
    let proxied = RepoCheckout {
        clone_url: "http://backend:8001/git/repo_1".to_string(),
        ..checkout("api", "github", true)
    };
    assert_eq!(clone_token(&config, &credentials, &proxied), None);
    let direct = RepoCheckout {
        clone_url: "https://github.com/acme/api.git".to_string(),
        ..checkout("api", "github", true)
    };
    assert_eq!(
        clone_token(&config, &credentials, &direct).as_deref(),
        Some("pod-token")
    );
}

fn credentials(pairs: &[(&str, &str)]) -> GitCredentials {
    GitCredentials::from_credentials(
        &pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect(),
    )
}

/// The probe answers from the resolver and the TCP stack, so both verdicts
/// are exercised against a real local socket rather than the public
/// network.
///
/// That means a listener that exists, and a port nothing is bound to.
#[tokio::test]
async fn probe_reports_reachable_and_unreachable_hosts() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let live = listener.local_addr().unwrap().port();
    assert!(
        host_reachable(
            &format!("https://127.0.0.1:{live}/acme/api.git"),
            false,
            probe()
        )
        .await
    );

    // Nothing is bound to `DEAD_PORT`, so the connect is refused outright.
    assert!(
        !host_reachable(
            &format!("https://127.0.0.1:{DEAD_PORT}/acme/api.git"),
            false,
            probe()
        )
        .await
    );
}

/// A multi-address host is reachable when *any* of its addresses connects.
///
/// On a dual-stack runner, `localhost` resolves `::1` before `127.0.0.1`,
/// so a `127.0.0.1`-only listener needs walking past the first address.
#[tokio::test]
async fn probe_walks_every_resolved_address() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    assert!(
        host_reachable(
            &format!("https://localhost:{port}/acme/api.git"),
            false,
            probe()
        )
        .await
    );
}

/// The retry loop must not sleep the budget away and then start an attempt
/// it cannot afford — the startup probe SIGKILLs the container mid-clone
/// and the error never reaches the termination log.
#[tokio::test]
async fn retry_stops_before_an_attempt_it_cannot_afford() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = RepoCheckout {
        uses_ssh: false,
        clone_url: format!("https://127.0.0.1:{DEAD_PORT}/acme/api.git"),
        repository_id: String::new(),
        branch: None,
        provider: "local".to_string(),
        mount_name: "api".to_string(),
        is_primary: true,
    };

    // A budget just over the first backoff: one retry is affordable, a second
    // is not. The production budget would spend 45s proving the same thing.
    let budget = Duration::from_secs(3);
    let started = Instant::now();
    let err = clone_with_retry(
        &repo,
        &tmp.path().join("api").display().to_string(),
        &NO_SOURCE,
        probe(),
        started + budget,
    )
    .await
    .expect_err("a clone against a dead port must fail");

    assert!(err.retryable);
    // Returned within the budget rather than overrunning it on a last attempt.
    assert!(
        started.elapsed() <= budget,
        "retry overran its budget: {:?}",
        started.elapsed()
    );
}

/// Answer every request with `status`, so a clone failure can be classified
/// against a service whose behaviour the test controls.
async fn serve_status(status: &'static str) -> u16 {
    serve_response(status, "").await
}

/// Answer every request with `status` and `body`.
async fn serve_response(status: &'static str, body: &'static str) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut discard = [0u8; 1024];
            let _ = socket.read(&mut discard).await;
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    port
}

fn repo_at(port: u16) -> RepoCheckout {
    RepoCheckout {
        uses_ssh: false,
        clone_url: format!("http://127.0.0.1:{port}/acme/api.git"),
        repository_id: String::new(),
        branch: None,
        provider: "local".to_string(),
        mount_name: "api".to_string(),
        is_primary: true,
    }
}

/// A service answering 5xx is reporting its own outage, not passing a
/// verdict on the repo. Reachability alone would call that permanent and
/// kill the sandbox for the length of a provider incident.
#[tokio::test]
async fn service_side_5xx_stays_retryable() {
    let port = serve_status("503 Service Unavailable").await;
    assert!(failure_is_retryable(&repo_at(port), None, probe()).await);
}

/// A 4xx never decides on its own — it falls through to reachability — but
/// a host that answers leaves the failure the repo's own, and permanent.
#[tokio::test]
async fn client_error_from_a_reachable_host_is_not_retryable() {
    let port = serve_status("404 Not Found").await;
    assert!(!failure_is_retryable(&repo_at(port), None, probe()).await);
}

/// An `info/refs` body as GitHub sends it: the service banner, then `HEAD`
/// carrying the capabilities, then one line per ref.
const ADVERTISEMENT: &str = concat!(
    "001e# service=git-upload-pack\n0000",
    "0155a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2 HEAD\0multi_ack side-band-64k ",
    "symref=HEAD:refs/heads/main agent=git/github-1\n",
    "003fa1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2 refs/heads/main\n",
    "003ea1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2 refs/tags/v1.0\n",
    "0000",
);

/// The incident shape: GitHub answers for the repo, then cancels the pack
/// stream mid-transfer. Nothing about the repo is wrong, so the failure is
/// retried rather than stopping the sandbox for good.
#[tokio::test]
async fn success_from_the_service_marks_the_failure_as_the_transfer() {
    let port = serve_response("200 OK", ADVERTISEMENT).await;
    let mut repo = repo_at(port);
    assert!(failure_is_retryable(&repo, None, probe()).await);

    repo.branch = Some("main".to_string());
    assert!(failure_is_retryable(&repo, None, probe()).await);
}

/// A 2xx that is not a git advertisement, such as a login page a redirect
/// landed on, proves nothing about the repo or its credentials.
#[tokio::test]
async fn success_with_a_non_git_body_is_not_retryable() {
    let port = serve_response("200 OK", "<html>Sign in</html>").await;
    assert!(!failure_is_retryable(&repo_at(port), None, probe()).await);
}

/// A 2xx says nothing about a branch the repo does not have, and no retry
/// makes it appear.
#[tokio::test]
async fn success_without_the_requested_branch_is_not_retryable() {
    let port = serve_response("200 OK", ADVERTISEMENT).await;
    let mut repo = repo_at(port);
    repo.branch = Some("deleted-feature".to_string());
    assert!(!failure_is_retryable(&repo, None, probe()).await);
}

#[test]
fn advertisement_names_branches_and_tags_only() {
    let advertisement = RepoAdvertisement {
        status: reqwest::StatusCode::OK,
        body: ADVERTISEMENT.to_string(),
    };
    assert!(advertisement.names_ref("main"));
    assert!(advertisement.names_ref("v1.0"));
    assert!(!advertisement.names_ref("HEAD"));
    assert!(!advertisement.names_ref("mai"));
    // A capability on the `HEAD` line is not a ref.
    assert!(!advertisement.names_ref("side-band-64k"));
}

/// A transfer failure that outlasts the budget exits as transient, so the
/// container restart retries the clone.
#[tokio::test]
async fn exhausted_budget_after_a_transfer_failure_stays_retryable() {
    let tmp = tempfile::tempdir().unwrap();
    // Every request, the pack fetch included, answers with the ref
    // advertisement: the service answered, the transfer did not.
    let port = serve_response("200 OK", ADVERTISEMENT).await;
    let err = clone_with_retry(
        &repo_at(port),
        &tmp.path().join("api").display().to_string(),
        &NO_SOURCE,
        probe(),
        Instant::now(),
    )
    .await
    .expect_err("a clone against a server that sends no pack must fail");

    assert!(err.retryable);
}

/// A URL with no host to probe teaches nothing about the pod's network, so
/// the clone failure has to stand on its own and stay permanent.
#[tokio::test]
async fn hostless_url_counts_as_reachable() {
    assert!(host_reachable("/srv/git/local-repo.git", false, probe()).await);
}

/// The probe targets the port the clone would actually use, and an injected
/// token in the URL's userinfo must not be mistaken for the host.
#[test]
fn clone_urls_resolve_to_the_host_and_port_the_clone_uses() {
    let port = |url: &str| clone_host_port(url, false);
    assert_eq!(
        port("https://github.com/owner/repo.git"),
        Some(("github.com".into(), 443))
    );
    assert_eq!(
        port("http://gitlab.internal/owner/repo.git"),
        Some(("gitlab.internal".into(), 80))
    );
    assert_eq!(
        port("https://forgejo.example.com:8443/org/repo.git"),
        Some(("forgejo.example.com".into(), 8443))
    );
    // scp-like: the ':' separates a path, not a port.
    assert_eq!(
        clone_host_port("git@github.com:owner/repo.git", true),
        Some(("github.com".into(), 22))
    );
    assert_eq!(
        port("ssh://git@example.com:2222/owner/repo.git"),
        Some(("example.com".into(), 2222))
    );
    // An injected token must not be read as the host.
    assert_eq!(
        port("https://x-access-token:ghs_tok@github.com/owner/repo.git"),
        Some(("github.com".into(), 443))
    );
    assert_eq!(port("/srv/git/local-repo.git"), None);
}

/// The incident shape: the host is unreachable and the retry budget is
/// gone, so the failure is reported as environmental (exit 1, the restart
/// retries) rather than as a permanent config error.
#[tokio::test]
async fn exhausted_budget_against_an_unreachable_host_stays_retryable() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = RepoCheckout {
        uses_ssh: false,
        clone_url: format!("https://127.0.0.1:{DEAD_PORT}/acme/api.git"),
        repository_id: String::new(),
        branch: None,
        provider: "local".to_string(),
        mount_name: "api".to_string(),
        is_primary: true,
    };

    let err = clone_with_retry(
        &repo,
        &tmp.path().join("api").display().to_string(),
        &NO_SOURCE,
        probe(),
        Instant::now(),
    )
    .await
    .expect_err("a clone against a dead port must fail");

    assert!(err.retryable);
    assert!(err.to_string().contains("api"), "message was: {err}");
}

/// A repo the pod can reach is judged on the clone itself: no amount of
/// retrying makes a missing repository appear, so it must not be reported
/// as a network fault.
#[tokio::test]
async fn reachable_host_reports_the_failure_as_its_own() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = RepoCheckout {
        uses_ssh: false,
        // No host to probe, so the probe cannot excuse the failure.
        clone_url: tmp.path().join("does-not-exist").display().to_string(),
        repository_id: String::new(),
        branch: None,
        provider: "local".to_string(),
        mount_name: "api".to_string(),
        is_primary: true,
    };

    let err = clone_with_retry(
        &repo,
        &tmp.path().join("api").display().to_string(),
        &NO_SOURCE,
        probe(),
        Instant::now() + Duration::from_mins(1),
    )
    .await
    .expect_err("cloning a path that does not exist must fail");

    assert!(!err.retryable);
}

/// A smart-HTTP git server over `git http-backend` serving `root`, which
/// accepts only the password currently in `token`.
async fn serve_git(root: PathBuf, token: Arc<Mutex<String>>) -> u16 {
    use axum::body::Body;
    use axum::extract::Request;
    use axum::http::{Response, StatusCode};
    use base64::Engine as _;
    use tokio::io::AsyncWriteExt;

    let handler = move |request: Request| {
        let root = root.clone();
        let token = Arc::clone(&token);
        async move {
            let (request, body) = request.into_parts();
            let password = request
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()?.strip_prefix("Basic "))
                .and_then(|encoded| {
                    base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .ok()
                })
                .and_then(|decoded| {
                    let decoded = String::from_utf8(decoded).ok()?;
                    Some(decoded.split_once(':')?.1.to_string())
                });
            let authorized = password.as_deref() == Some(token.lock().unwrap().as_str());
            if !authorized {
                return Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header("www-authenticate", "Basic realm=\"test\"")
                    .body(Body::empty())
                    .unwrap();
            }
            let header = |name: &str| {
                request
                    .headers
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_string()
            };
            let mut child = tokio::process::Command::new("git")
                .arg("http-backend")
                .env("GIT_PROJECT_ROOT", &root)
                .env("GIT_HTTP_EXPORT_ALL", "1")
                .env("REMOTE_USER", "test")
                .env("REQUEST_METHOD", request.method.as_str())
                .env("PATH_INFO", request.uri.path())
                .env("QUERY_STRING", request.uri.query().unwrap_or_default())
                .env("CONTENT_TYPE", header("content-type"))
                .env("HTTP_CONTENT_ENCODING", header("content-encoding"))
                .env("GIT_PROTOCOL", header("git-protocol"))
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let body = axum::body::to_bytes(body, usize::MAX).await.unwrap();
            let mut stdin = child.stdin.take().unwrap();
            stdin.write_all(&body).await.unwrap();
            drop(stdin);
            let output = child.wait_with_output().await.unwrap();

            // CGI output: headers, a blank line, then the body.
            let split = output
                .stdout
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap();
            let mut response = Response::builder();
            for line in String::from_utf8_lossy(&output.stdout[..split]).lines() {
                let (name, value) = line.split_once(": ").unwrap();
                if name.eq_ignore_ascii_case("status") {
                    let code = value.split(' ').next().unwrap().parse::<u16>().unwrap();
                    response = response.status(code);
                } else {
                    response = response.header(name, value);
                }
            }
            response
                .body(Body::from(output.stdout[split + 4..].to_vec()))
                .unwrap()
        }
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, axum::Router::new().fallback(handler))
            .await
            .unwrap();
    });
    port
}

/// A bare `acme/api.git` under a fresh server root, served behind `token`.
async fn hosted_repo(tmp: &Path, token: &Arc<Mutex<String>>) -> String {
    let source = tmp.join("source");
    init_local_source_repo(&source);
    let root = tmp.join("served");
    std::fs::create_dir_all(root.join("acme")).unwrap();
    run_git_sync(
        tmp,
        &[
            "clone",
            "--quiet",
            "--bare",
            &source.display().to_string(),
            &root.join("acme/api.git").display().to_string(),
        ],
    );
    let port = serve_git(root, Arc::clone(token)).await;
    format!("http://127.0.0.1:{port}")
}

fn git_output(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new("git")
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(args)
        .output()
        .expect("git command runs")
}

/// The incident: a GitHub checkout cloned with its token in `origin` sent
/// that token after it expired, and no helper was ever asked.
// Multi-threaded: the test's blocking `git` calls would otherwise starve
// the server it is talking to.
#[tokio::test(flavor = "multi_thread")]
async fn a_broker_origin_clone_keeps_no_token_and_fetches_after_a_rotation() {
    let tmp = tempfile::tempdir().unwrap();
    let token = Arc::new(Mutex::new("ghs_first".to_string()));
    let origin = hosted_repo(tmp.path(), &token).await;
    let url = format!("{origin}/acme/api.git");
    let dest = tmp.path().join("api");
    let repo = RepoCheckout {
        clone_url: url.clone(),
        ..checkout("api", "github", true)
    };

    clone_checkout(
        &repo,
        &dest.display().to_string(),
        &CloneSource {
            token: Some("ghs_first"),
            github_upstream: None,
            broker_origins: std::slice::from_ref(&origin),
        },
    )
    .await
    .unwrap();

    let config = std::fs::read_to_string(dest.join(".git/config")).unwrap();
    assert!(!config.contains("ghs_first"), "{config}");
    assert!(!config.contains("[remote \"github\"]"), "{config}");
    assert_eq!(
        String::from_utf8_lossy(&git_output(&dest, &["remote", "get-url", "origin"]).stdout).trim(),
        url
    );

    // The broker answers with whatever token is live, as this helper does.
    *token.lock().unwrap() = "ghs_second".to_string();
    let fetch = git_output(
        &dest,
        &[
            "-c",
            "credential.helper=!f() { echo username=oauth2; echo password=ghs_second; }; f",
            "fetch",
            "origin",
        ],
    );
    assert!(
        fetch.status.success(),
        "{}",
        String::from_utf8_lossy(&fetch.stderr)
    );
}

/// A proxied GitHub checkout records the repository it stands in for, so
/// `gh` and the push watcher can name it.
#[tokio::test(flavor = "multi_thread")]
async fn a_proxied_github_clone_records_its_github_remote() {
    let tmp = tempfile::tempdir().unwrap();
    let token = Arc::new(Mutex::new("task-token".to_string()));
    let origin = hosted_repo(tmp.path(), &token).await;
    let url = format!("{origin}/acme/api.git");
    let dest = tmp.path().join("api");
    let repo = RepoCheckout {
        clone_url: url.clone(),
        ..checkout("api", "github", true)
    };

    // The clone-token helper stands in for the broker answering the proxy.
    clone_checkout(
        &repo,
        &dest.display().to_string(),
        &CloneSource {
            token: Some("task-token"),
            github_upstream: Some("https://github.com/acme/api.git"),
            broker_origins: std::slice::from_ref(&origin),
        },
    )
    .await
    .unwrap();

    let get = |key: &str| {
        String::from_utf8_lossy(&git_output(&dest, &["config", "--get", key]).stdout)
            .trim()
            .to_string()
    };
    assert_eq!(get("remote.origin.url"), url);
    assert_eq!(get("remote.github.url"), "https://github.com/acme/api.git");
    assert_eq!(get("remote.github.pushurl"), url);
    assert_eq!(
        get("remote.github.fetch"),
        "+refs/heads/*:refs/remotes/github/*"
    );
    assert_eq!(get("remote.github.skipFetchAll"), "true");
    let remotes = git_output(&dest, &["remote"]);
    assert!(
        String::from_utf8_lossy(&remotes.stdout).contains("github"),
        "{}",
        String::from_utf8_lossy(&remotes.stderr)
    );
}

#[test]
fn inject_token_basic() {
    assert_eq!(
        inject_token_into_url("https://github.com/owner/repo.git", "tok123"),
        "https://x-access-token:tok123@github.com/owner/repo.git"
    );
}

#[test]
fn inject_token_replaces_existing_credentials() {
    assert_eq!(
        inject_token_into_url("https://old:cred@github.com/owner/repo.git", "new_token"),
        "https://x-access-token:new_token@github.com/owner/repo.git"
    );
}

/// The backend hands the sandbox a credential-free clone URL (userinfo
/// stripped by `pod_builder::redact_url_userinfo`); cloning
/// re-authenticates by injecting the provider token.
#[test]
fn redacted_url_authenticates_via_injected_token() {
    assert_eq!(
        inject_token_into_url("https://github.com/owner/repo.git", "ghs_tok"),
        "https://x-access-token:ghs_tok@github.com/owner/repo.git"
    );
    assert_eq!(
        inject_token_into_url("https://gitlab.com/group/repo.git", "glpat_tok"),
        "https://x-access-token:glpat_tok@gitlab.com/group/repo.git"
    );
}

#[test]
fn inject_token_ssh_passthrough() {
    let ssh_url = "git@github.com:owner/repo.git";
    assert_eq!(inject_token_into_url(ssh_url, "tok"), ssh_url);
}

const NO_SOURCE: CloneSource<'static> = CloneSource {
    token: None,
    github_upstream: None,
    broker_origins: &[],
};

fn probe() -> HttpTimeouts {
    crate::config::SandboxHttpConfig::default().git_probe
}
