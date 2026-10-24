//! GitHub App registration tests against Postgres.
//!
//! Proves what a unit test cannot: a registration made through one registry
//! instance is visible to another sharing the database (the two-replica case).
//! GitHub is stubbed at the REST root [`stub_github`] returns.

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};

use axum::Json;
use axum::routing::get;
use moatless_types::auth::GitHubAppConfig;
use moatless_types::secret::SecretsConfig;
use sqlx::PgPool;

use super::PgAdapterAppDao;
use super::{AdapterAppDao, GitHubAppAuditEntry, GitHubAppRecord, NewGitHubApp};
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::GitHubAppService;
use crate::secret::service::{LocalEnvelopeSecretCipher, SecretCipher};

/// A throwaway key, so a service built from a stored record is a real one.
const TEST_PEM: &str = include_str!("../auth/github/testdata/github_app_test_key.pem");

static STUB: OnceLock<SocketAddr> = OnceLock::new();

/// The REST root of a stub for GitHub's `GET /app`, on its own thread and
/// runtime so the address outlives the test that started it.
fn stub_github() -> String {
    format!("http://{}", stub_github_addr())
}

fn stub_github_addr() -> SocketAddr {
    *STUB.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let app = axum::Router::new().route(
                    "/app",
                    get(|| async {
                        Json(serde_json::json!({
                            "id": 4242,
                            "slug": "reviewer",
                            "name": "Reviewer",
                        }))
                    }),
                );
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(listener.local_addr().unwrap()).unwrap();
                axum::serve(listener, app).await.unwrap();
            });
        });
        rx.recv().expect("GitHub stub reports its address")
    })
}

fn cipher() -> Arc<dyn SecretCipher> {
    LocalEnvelopeSecretCipher::from_config(&SecretsConfig {
        encryption_key: Some("0123456789abcdef0123456789abcdef".into()),
        kms_key_id: None,
    })
    .expect("cipher")
}

/// A registry with no login App, so every lookup goes to storage.
fn registry(pool: &PgPool) -> GitHubAppRegistry {
    GitHubAppRegistry::new(
        None,
        Some(Arc::new(PgAdapterAppDao(pool.clone()))),
        Some(cipher()),
        stub_github(),
        moatless_core::config::HttpConfig::default().github,
    )
}

fn app_config(key: &str, app_id: u64) -> GitHubAppConfig {
    GitHubAppConfig {
        key: key.to_string(),
        app_id,
        private_key: TEST_PEM.to_string(),
        client_id: None,
        client_secret: None,
        default_installation_id: None,
    }
}

/// Prove and store an App the way the register handler does.
async fn register(pool: &PgPool, key: &str, app_id: u64) -> GitHubAppRecord {
    let cipher = cipher();
    let proved = GitHubAppService::from_app_config(
        &app_config(key, app_id),
        &stub_github(),
        moatless_core::config::HttpConfig::default().github,
    )
    .expect("key parses")
    .prove()
    .await
    .expect("stub accepts the key");
    let record = GitHubAppRecord::seal(
        NewGitHubApp {
            app_key: key.to_string(),
            app_id: proved.app_id,
            name: proved.name,
            slug: proved.slug,
            bot_login: proved.bot_login,
            private_key_pem: TEST_PEM.to_string(),
            default_installation_id: None,
        },
        cipher.as_ref(),
        None,
    )
    .expect("seal");
    PgAdapterAppDao(pool.clone())
        .insert_github_app(&record, "admin-1")
        .await
        .expect("insert");
    record
}

#[sqlx::test]
async fn a_proved_app_is_stored_with_the_identity_github_reported(pool: PgPool) {
    let record = register(&pool, "review-bot", 4242).await;

    assert_eq!(record.name, "Reviewer");
    assert_eq!(record.slug, "reviewer");
    assert_eq!(record.bot_login, "reviewer[bot]");

    let stored = PgAdapterAppDao(pool.clone())
        .find_github_app_by_key("review-bot")
        .await
        .unwrap()
        .expect("stored");
    // The key survives the round trip through the database, and is the one
    // that was supplied — not something that merely decrypts.
    assert_eq!(stored.private_key_pem(cipher().as_ref()).unwrap(), TEST_PEM);
    assert_eq!(stored.key_fingerprint, record.key_fingerprint);
}

#[sqlx::test]
async fn a_key_that_names_a_different_app_is_refused_and_nothing_is_stored(pool: PgPool) {
    // The stub always answers as App 4242; claiming 9999 is the "this key
    // belongs to a different App" case.
    let error = GitHubAppService::from_app_config(
        &app_config("review-bot", 9999),
        &stub_github(),
        moatless_core::config::HttpConfig::default().github,
    )
    .expect("key parses")
    .prove()
    .await
    .expect_err("a mismatched App id must be refused");

    assert!(error.to_string().contains("different App"), "{error}");
    assert!(
        PgAdapterAppDao(pool)
            .find_github_app_by_key("review-bot")
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test]
async fn a_second_registration_under_the_same_name_is_a_conflict(pool: PgPool) {
    let record = register(&pool, "review-bot", 4242).await;

    // Overwriting would silently repoint every bot user naming the key.
    let error = PgAdapterAppDao(pool)
        .insert_github_app(&record, "admin-2")
        .await
        .expect_err("a taken name must conflict");
    assert!(error.to_string().contains("already registered"), "{error}");
}

#[sqlx::test]
async fn an_app_registered_on_one_backend_is_usable_on_another(pool: PgPool) {
    // Two registries over one database: the two-replica case without two
    // processes. Neither is told anything; the second simply reads.
    let backend_a = registry(&pool);
    let backend_b = registry(&pool);

    assert!(backend_b.get("review-bot").await.unwrap().is_none());
    register(&pool, "review-bot", 4242).await;

    assert!(backend_a.get("review-bot").await.unwrap().is_some());
    assert!(backend_b.get("review-bot").await.unwrap().is_some());
}

#[sqlx::test]
async fn re_registering_a_removed_key_does_not_keep_the_deleted_signing_key_alive(pool: PgPool) {
    // The security-relevant case: a replacement record also starts at revision
    // 1, so a backend that cached the removed App must not keep minting tokens
    // with a signing key an admin has revoked.
    let backend = registry(&pool);
    let removed = register(&pool, "review-bot", 4242).await;
    let before = backend.get("review-bot").await.unwrap().expect("app");

    assert!(
        PgAdapterAppDao(pool.clone())
            .delete_github_app("review-bot")
            .await
            .unwrap()
    );
    let replacement = register(&pool, "review-bot", 4242).await;
    assert_ne!(replacement.id, removed.id);
    assert_eq!(replacement.revision, removed.revision);

    let after = backend.get("review-bot").await.unwrap().expect("app");
    assert!(!Arc::ptr_eq(&before, &after));
}

#[sqlx::test]
async fn a_rotation_on_one_backend_reaches_another_without_coordination(pool: PgPool) {
    let backend_a = registry(&pool);
    let backend_b = registry(&pool);
    let record = register(&pool, "review-bot", 4242).await;

    let before_a = backend_a.get("review-bot").await.unwrap().expect("app");
    let before_b = backend_b.get("review-bot").await.unwrap().expect("app");

    let rotated = GitHubAppRecord::seal(
        NewGitHubApp {
            app_key: record.app_key.clone(),
            app_id: record.app_id,
            name: record.name.clone(),
            slug: record.slug.clone(),
            bot_login: record.bot_login.clone(),
            private_key_pem: format!("{TEST_PEM}\n"),
            default_installation_id: None,
        },
        cipher().as_ref(),
        Some(record.id.clone()),
    )
    .expect("seal");
    assert!(
        PgAdapterAppDao(pool.clone())
            .replace_github_app(&rotated, "admin-1")
            .await
            .unwrap()
    );

    // Neither backend was notified: both notice because the revision moved.
    let after_a = backend_a.get("review-bot").await.unwrap().expect("app");
    let after_b = backend_b.get("review-bot").await.unwrap().expect("app");
    assert!(!Arc::ptr_eq(&before_a, &after_a));
    assert!(!Arc::ptr_eq(&before_b, &after_b));

    let stored = PgAdapterAppDao(pool)
        .find_github_app_by_key("review-bot")
        .await
        .unwrap()
        .expect("stored");
    assert_eq!(stored.revision, record.revision + 1);
}

#[sqlx::test]
async fn removing_an_app_stops_it_answering_everywhere(pool: PgPool) {
    let backend_a = registry(&pool);
    let backend_b = registry(&pool);
    register(&pool, "review-bot", 4242).await;

    assert!(backend_a.get("review-bot").await.unwrap().is_some());
    assert!(backend_b.get("review-bot").await.unwrap().is_some());

    assert!(
        PgAdapterAppDao(pool.clone())
            .delete_github_app("review-bot")
            .await
            .unwrap()
    );

    assert!(backend_a.get("review-bot").await.unwrap().is_none());
    assert!(backend_b.get("review-bot").await.unwrap().is_none());
}

#[sqlx::test]
async fn administration_is_recorded_against_whoever_did_it(pool: PgPool) {
    let record = register(&pool, "review-bot", 4242).await;
    let store = PgAdapterAppDao(pool.clone());
    store
        .record_github_app_audit(GitHubAppAuditEntry {
            github_app_id: Some(&record.id),
            app_key: &record.app_key,
            actor_id: "admin-1",
            action: "register",
            result: "ok",
            detail: None,
        })
        .await
        .unwrap();

    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT actor_id, action, result FROM adapter_app_audit_events WHERE app_key = $1",
    )
    .bind("review-bot")
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(
        rows,
        vec![("admin-1".into(), "register".into(), "ok".into())]
    );
}
