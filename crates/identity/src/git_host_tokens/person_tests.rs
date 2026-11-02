//! A bot's installation token capped by the person whose work it runs.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};

use super::*;
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::GitHubAppService;
use crate::auth::provider_token_auth_dao::MockProviderTokenAuthDao;
use crate::auth::test_support::{StubProviderTokens, github_row};
use crate::secret::test_support::service_with;
use moatless_types::auth::GitHubAppConfig;

const TEST_PEM: &str = include_str!("../auth/github/testdata/github_app_test_key.pem");

type Requests = Arc<Mutex<Vec<serde_json::Value>>>;

/// GitHub for one installation that grants `administration`, and three people:
/// `ada` administers `acme/repo`, `bob` reads it, anyone else has no access.
async fn github() -> (SocketAddr, Requests) {
    let requests = Requests::default();
    let app = axum::Router::new()
        .route(
            "/orgs/acme/installation",
            get(|| async { Json(serde_json::json!({ "id": 7 })) }),
        )
        .route(
            "/app/installations/7/access_tokens",
            post(
                |State(requests): State<Requests>, body: axum::body::Bytes| async move {
                    let request: serde_json::Value =
                        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
                    requests.lock().unwrap().push(request.clone());
                    let (token, permissions) = match request.get("permissions") {
                        Some(permissions) => ("ghs_capped", permissions.clone()),
                        None => (
                            "ghs_wide",
                            serde_json::json!({
                                "administration": "write",
                                "contents": "write",
                                "metadata": "read",
                                "pull_requests": "write",
                            }),
                        ),
                    };
                    (
                        StatusCode::CREATED,
                        Json(serde_json::json!({
                            "token": token,
                            "expires_at": "2099-01-01T00:00:00Z",
                            "permissions": permissions,
                        })),
                    )
                },
            ),
        )
        .route(
            "/repos/acme/repo/collaborators/{login}/permission",
            get(|Path(login): Path<String>| async move {
                match login.as_str() {
                    "ada" => Json(serde_json::json!({ "permission": "admin" })).into_response(),
                    "bob" => Json(serde_json::json!({ "permission": "read" })).into_response(),
                    _ => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        )
        .with_state(requests.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, requests)
}

fn person_row(login: &str) -> ProviderToken {
    ProviderToken {
        provider_login: Some(login.to_string()),
        ..github_row(AUTH_METHOD_APP_USER, "ghu_person")
    }
}

/// `bot` is connected through the App installation; `people` hold their own
/// rows.
async fn issuer(people: &[(&str, ProviderToken)]) -> (GitHostTokens, Requests) {
    let (addr, requests) = github().await;
    let base = format!("http://{addr}");
    let config = GitHubAppConfig {
        key: "review-bot".to_string(),
        app_id: 42,
        private_key: TEST_PEM.to_string(),
        client_id: None,
        client_secret: None,
        default_installation_id: None,
    };
    let app = GitHubAppService::from_app_config(
        &config,
        &base,
        moatless_core::config::HttpConfig::default().github,
    )
    .expect("test key parses");
    let registry = GitHubAppRegistry::new(
        Some(Arc::new(app)),
        None,
        None,
        base,
        moatless_core::config::HttpConfig::default().github,
    );

    let mut bot = github_row(
        AUTH_METHOD_APP_INSTALLATION,
        crate::auth::provider_tokens::APP_INSTALLATION_TOKEN_PLACEHOLDER,
    );
    bot.provider_user_id = Some("review-bot".to_string());
    let rows = people.iter().fold(
        StubProviderTokens::default().with_row_for("bot", bot),
        |rows, (user, row)| rows.with_row_for(user, row.clone()),
    );
    let tokens = GitHostTokens::new(
        Arc::new(rows),
        Arc::new(MockProviderTokenAuthDao::new()),
        None,
        Arc::new(registry),
        Arc::new(service_with(vec![])),
    );
    (tokens, requests)
}

fn repo() -> GitHostAccount {
    GitHostAccount::Github {
        owner: "acme".into(),
        repository: Some("repo".into()),
    }
}

fn bot_for(person: &str) -> Actor {
    Actor::of("bot").on_behalf_of(Some(person))
}

/// The permissions the last mint asked GitHub for.
fn last_permissions(requests: &Requests) -> BTreeMap<String, String> {
    let requests = requests.lock().unwrap();
    let last = requests.last().expect("a token was minted");
    assert_eq!(last["repositories"], serde_json::json!(["repo"]));
    serde_json::from_value(last["permissions"].clone()).unwrap()
}

fn permissions(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

#[tokio::test]
async fn an_admin_running_a_bot_gets_a_writer_token_for_the_one_repository() {
    let (tokens, requests) = issuer(&[("u_ada", person_row("ada"))]).await;

    let token = tokens.token(&bot_for("u_ada"), &repo()).await.unwrap();

    assert_eq!(token, "ghs_capped");
    assert_eq!(
        last_permissions(&requests),
        permissions(&[
            ("contents", "write"),
            ("metadata", "read"),
            ("pull_requests", "write"),
        ]),
        "administration is never passed on, even to an admin"
    );
}

#[tokio::test]
async fn a_reader_running_a_bot_gets_a_read_only_token() {
    let (tokens, requests) = issuer(&[("u_bob", person_row("bob"))]).await;

    tokens.token(&bot_for("u_bob"), &repo()).await.unwrap();

    assert_eq!(
        last_permissions(&requests),
        permissions(&[
            ("contents", "read"),
            ("metadata", "read"),
            ("pull_requests", "read"),
        ])
    );
}

#[tokio::test]
async fn a_person_without_access_gets_no_token() {
    let (tokens, _) = issuer(&[("u_eve", person_row("eve"))]).await;

    let error = tokens.token(&bot_for("u_eve"), &repo()).await.unwrap_err();

    match error {
        GitHostTokenError::Forbidden { actor, .. } => {
            assert_eq!(
                actor.user_id(),
                "u_eve",
                "the refusal names whose access is missing"
            );
        }
        other => panic!("expected Forbidden, got {other:?}"),
    }
}

#[tokio::test]
async fn a_person_who_never_connected_github_is_asked_to() {
    let (tokens, _) = issuer(&[]).await;

    let error = tokens.token(&bot_for("u_new"), &repo()).await.unwrap_err();

    match error {
        GitHostTokenError::NoCredential { actor, .. } => assert_eq!(actor.user_id(), "u_new"),
        other => panic!("expected NoCredential, got {other:?}"),
    }
}

#[tokio::test]
async fn a_person_never_gets_a_token_for_a_whole_account() {
    let (tokens, _) = issuer(&[("u_ada", person_row("ada"))]).await;
    let account = GitHostAccount::Github {
        owner: "acme".into(),
        repository: None,
    };

    let error = tokens.token(&bot_for("u_ada"), &account).await.unwrap_err();

    assert!(
        matches!(error, GitHostTokenError::Forbidden { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_bot_running_its_own_work_keeps_the_installation_token() {
    let (tokens, _) = issuer(&[]).await;

    let token = tokens.token(&Actor::of("bot"), &repo()).await.unwrap();

    assert_eq!(token, "ghs_wide");
}

#[tokio::test]
async fn an_owner_the_app_is_not_installed_on_is_out_of_reach_not_broken() {
    let (tokens, _) = issuer(&[("ada", person_row("ada"))]).await;
    let elsewhere = |repository: Option<&str>| GitHostAccount::Github {
        owner: "openai".into(),
        repository: repository.map(str::to_string),
    };

    for (actor, account) in [
        (Actor::of("bot"), elsewhere(None)),
        (Actor::of("bot"), elsewhere(Some("plugins"))),
        (bot_for("ada"), elsewhere(Some("plugins"))),
    ] {
        let error = tokens.token(&actor, &account).await.unwrap_err();
        assert!(
            matches!(error, GitHostTokenError::Forbidden { .. }),
            "{actor}: {error:?}"
        );
    }
}
