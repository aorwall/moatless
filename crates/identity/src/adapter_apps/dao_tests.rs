//! GitHub App registration against Postgres.
//!
//! Two things are proved here that a unit test cannot: that a registration
//! made through one registry instance is visible to another sharing the same
//! database — the two-replica case, without two processes — and that the
//! one-time carry-over of `GITHUB_APPS_JSON` converges to a single record when
//! several backends run it at once.
//!
//! GitHub itself is stubbed through `GITHUB_API_BASE_URL`, which
//! `moatless_core::github` reads on every client build. That variable is
//! process-global, so [`stub_github`] takes a lock for as long as the stub must
//! answer.

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};

use axum::Json;
use axum::routing::get;
use moatless_types::auth::GitHubAppConfig;
use moatless_types::secret::SecretsConfig;
use sqlx::PgPool;

use super::PgAdapterAppDao;
use super::{AdapterAppDao, ENV_CARRY_OVER, GitHubAppAuditEntry, GitHubAppRecord, NewGitHubApp};
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::GitHubAppService;
use crate::secret::service::{LocalEnvelopeSecretCipher, SecretCipher};

/// A throwaway key, so a service built from a stored record is a real one.
const TEST_PEM: &str = include_str!("../auth/github/testdata/github_app_test_key.pem");

static STUB: OnceLock<SocketAddr> = OnceLock::new();

/// Exclusive, restoring ownership of `GITHUB_API_BASE_URL`, pointed at a stub
/// standing in for GitHub's `GET /app`. The proof call is the whole of what
/// these tests need from GitHub.
///
/// The address behind it is cached for the whole process, so the server has to
/// outlive the test that happened to start it. It therefore gets a thread and a
/// runtime of its own rather than a `tokio::spawn` onto the caller's: under a
/// runner that gives each test its own runtime and drops it at the end (plain
/// `cargo test`, one process, thread-per-test), spawning here would leave every
/// later test holding the address of a listener that died with the first test's
/// runtime.
async fn stub_github() -> GithubApiBaseGuard {
    static LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

    let guard = LOCK.lock().await;
    let previous = std::env::var_os("GITHUB_API_BASE_URL");
    std::env::set_var(
        "GITHUB_API_BASE_URL",
        format!("http://{}", stub_github_addr()),
    );
    GithubApiBaseGuard {
        _lock: guard,
        previous,
    }
}

struct GithubApiBaseGuard {
    _lock: tokio::sync::MutexGuard<'static, ()>,
    previous: Option<std::ffi::OsString>,
}

impl Drop for GithubApiBaseGuard {
    fn drop(&mut self) {
        if let Some(value) = self.previous.take() {
            std::env::set_var("GITHUB_API_BASE_URL", value);
        } else {
            std::env::remove_var("GITHUB_API_BASE_URL");
        }
    }
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
    let _github_api = stub_github().await;
    let cipher = cipher();
    let proved = GitHubAppService::from_app_config(&app_config(key, app_id))
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
    let _github_api = stub_github().await;
    // The stub always answers as App 4242; claiming 9999 is the "this key
    // belongs to a different App" case.
    let error = GitHubAppService::from_app_config(&app_config("review-bot", 9999))
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
async fn the_carry_over_produces_one_record_when_several_backends_run_it(pool: PgPool) {
    let _github_api = stub_github().await;
    let entries = vec![app_config("env-bot", 4242)];

    let backends: Vec<GitHubAppRegistry> = (0..3).map(|_| registry(&pool)).collect();
    // Started together, as a rollout starts them.
    futures::future::join_all(
        backends
            .iter()
            .map(|backend| backend.carry_over_env_apps(&entries)),
    )
    .await;

    let stored = PgAdapterAppDao(pool.clone())
        .list_github_apps()
        .await
        .unwrap();
    assert_eq!(stored.len(), 1, "expected exactly one carried-over App");
    assert_eq!(stored[0].app_key, "env-bot");
    assert_eq!(stored[0].bot_login, "reviewer[bot]");
}

#[sqlx::test]
async fn a_deleted_carried_over_app_is_not_resurrected_by_a_restart(pool: PgPool) {
    let _github_api = stub_github().await;
    let entries = vec![app_config("env-bot", 4242)];

    registry(&pool).carry_over_env_apps(&entries).await;
    assert!(
        PgAdapterAppDao(pool.clone())
            .delete_github_app("env-bot")
            .await
            .unwrap()
    );

    // Whether the carry-over has run is recorded in its own right, so a
    // restart with the variable still set does not undo the deletion.
    registry(&pool).carry_over_env_apps(&entries).await;

    assert!(
        PgAdapterAppDao(pool.clone())
            .find_github_app_by_key("env-bot")
            .await
            .unwrap()
            .is_none()
    );
    let claimed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM adapter_app_migrations WHERE name = $1")
            .bind(ENV_CARRY_OVER)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(claimed, 1, "the carry-over must stay recorded as done");
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
