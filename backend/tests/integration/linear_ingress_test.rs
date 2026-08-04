#![cfg(feature = "db-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test fixtures assert on invariants a broken fixture must fail loudly on, here \
              and in the shared `common` module compiled into this binary; both lints only \
              fire when the db-tests feature is enabled"
)]
//! The seam stage 15 exists to create, driven directly.
//!
//! Everything below calls [`ingress::receive`] with a signature and a body.
//! There is no `Request`, no router, no axum extractor — which is the whole
//! point: before this stage the only way to reach any Linear ingress decision
//! was to deliver an HTTP request to `linear_webhook`, so no test could name a
//! decision without also exercising the transport around it.
//!
//! What `receive` returns is the vocabulary the route maps to a status and a
//! metric row, and `linear_webhook_metrics_test.rs` asserts that mapping. This
//! file asserts the layer under it: which outcome each delivery reaches, which
//! reason labels it, what the admission committed, and which `IngressError`
//! variant a refusal raises.

use crate::common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use sqlx::PgPool;

use moatless_vibe_backend::adapters::linear::adapter::LinearAdapter;
use moatless_vibe_backend::adapters::linear::delivery;
use moatless_vibe_backend::adapters::linear::ingress::{
    reason, receive, IngressError, Outcome, Rejection,
};
use moatless_vibe_backend::adapters::models::{RoutingMode, SubscriptionMatch};
use moatless_vibe_backend::adapters::test_support::ProviderStub;
use moatless_vibe_backend::adapters::Adapters;
use moatless_vibe_backend::repository::dao as repository_dao;
use moatless_vibe_backend::scope::Scope;
use moatless_vibe_backend::state::AppState;

use common::{
    linear_config, make_repository_row, seed_team, test_app_state_with_config, test_config, USER_1,
};

const SECRET: &str = "linear-webhook-secret";
const ORG: &str = "org-1";
const TEAM: &str = "team-1";
const ISSUE: &str = "issue-1";
const CONNECTION: &str = "conn-linear-1";
const SESSION: &str = "session-1";

type HmacSha256 = Hmac<Sha256>;

// =========================================================================
// Fixtures
// =========================================================================

/// A Linear that answers every operation successfully, so the one call the
/// created-session path makes before the acknowledgment is observable rather
/// than merely attempted.
async fn linear_stub() -> ProviderStub {
    ProviderStub::start(|_path| {
        serde_json::json!({
            "data": {
                "agentActivityCreate": {"success": true, "agentActivity": {"id": "activity-stub"}},
            }
        })
    })
    .await
}

fn replica(pool: &PgPool, stub: &ProviderStub) -> AppState {
    let mut adapters = Adapters::new();
    let mut config = test_config();
    config.linear = linear_config(Some(SECRET), Some(stub.base_url()));
    adapters.register_linear(Arc::new(LinearAdapter::new(&config.linear, pool.clone())));
    test_app_state_with_config(pool.clone(), Some(Arc::new(adapters)), config)
}

/// Call `receive` the way the route does, minus the route.
async fn deliver(state: &AppState, envelope: &serde_json::Value) -> Result<Outcome, Rejection> {
    let body = serde_json::to_vec(envelope).expect("serialize envelope");
    deliver_signed_bytes(state, body, true).await
}

async fn deliver_signed_bytes(
    state: &AppState,
    body: Vec<u8>,
    sign: bool,
) -> Result<Outcome, Rejection> {
    let signature = if sign {
        let mut mac = HmacSha256::new_from_slice(SECRET.as_bytes()).expect("hmac key");
        mac.update(&body);
        hex::encode(mac.finalize().into_bytes())
    } else {
        String::new()
    };
    let provider = state
        .adapters
        .as_ref()
        .expect("adapters registered")
        .linear()
        .expect("Linear adapter registered");
    receive(state, provider, &signature, &body).await
}

async fn seed_connection(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO adapter_connections \
         (id, adapter_kind, connection_kind, external_account_id, created_by, scope, owner_user_id) \
         VALUES ($1, 'linear', 'org', $2, $3, 'user', $3)",
    )
    .bind(CONNECTION)
    .bind(ORG)
    .bind(USER_1)
    .execute(pool)
    .await
    .expect("seed Linear connection");
}

/// The workspace OAuth token every Linear call resolves before it reaches HTTP.
/// Without it the heartbeat never leaves the process and the stub sees nothing.
async fn seed_workspace_token(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO provider_tokens \
         (provider, auth_method, access_token, token_type, connected_at, updated_at, \
          scope, owner_user_id, expires_at) \
         VALUES ('linear', 'oauth', 'stub-access-token', 'Bearer', NOW(), NOW(), \
                 'global', NULL, $1)",
    )
    .bind(Utc::now() + Duration::days(30))
    .execute(pool)
    .await
    .expect("seed Linear workspace token");
}

/// An active Linear Subscription for the team, which is what a created session
/// is gated on.
async fn seed_rule(state: &AppState, pool: &PgPool) {
    seed_team(pool, &format!("personal-{USER_1}")).await;
    let mut repo = make_repository_row("repo-1", USER_1);
    repo.provider = None;
    repo.remote_url = None;
    repository_dao::insert(pool, &repo)
        .await
        .expect("seed repository");

    let now = Utc::now();
    state
        .loops
        .upsert_subscription(&SubscriptionMatch {
            id: "trigger-linear-1".to_string(),
            adapter_kind: "linear".to_string(),
            connection_id: CONNECTION.to_string(),
            source_matcher: TEAM.to_string(),
            source_name: Some("Engineering".to_string()),
            routing_mode: RoutingMode::BySubject,
            event_filter: serde_json::json!([]),
            repository_id: "repo-1".to_string(),
            prompt_template: None,
            task_name: None,
            tag_ids: vec![],
            // A Subscription with no run-as user is invisible to every lookup
            // that resolves rules — `run_as_user_id IS NOT NULL` is in the
            // query. Without it the rule exists and matches nothing.
            run_as_user_id: Some(USER_1.to_string()),
            agent_type: "claude-code".to_string(),
            inactive_ttl_days: None,
            allow_direct_send: false,
            close_task_on_close: false,
            adapter_config: None,
            ongoing_task_id: None,
            notify_on_success: false,
            active: true,
            created_by: USER_1.to_string(),
            scope: Scope::User,
            owner_user_id: Some(USER_1.to_string()),
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("seed Linear rule");
}

async fn admitted_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM adapter_delivery")
        .fetch_one(pool)
        .await
        .expect("count admitted records")
}

// =========================================================================
// Envelopes
// =========================================================================

fn comment_envelope(action: &str, comment_id: &str) -> serde_json::Value {
    serde_json::json!({
        "action": action,
        "type": "Comment",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "webhookTimestamp": 1_700_000_000_000i64,
        "data": {
            "id": comment_id,
            "body": "please look at this",
            "issueId": ISSUE,
        },
    })
}

fn issue_envelope(timestamped: bool) -> serde_json::Value {
    let mut envelope = serde_json::json!({
        "action": "update",
        "type": "Issue",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "updatedFrom": {"stateId": "s0"},
        "data": {
            "id": ISSUE,
            "identifier": "ENG-7",
            "title": "Fix it",
            "teamId": TEAM,
            "state": {"id": "s1", "name": "Done", "type": "completed"},
        },
    });
    if timestamped {
        envelope["webhookTimestamp"] = serde_json::json!(1_700_000_000_000i64);
    }
    envelope
}

fn session_envelope(action: &str) -> serde_json::Value {
    serde_json::json!({
        "action": action,
        "type": "AgentSessionEvent",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "webhookTimestamp": 1_700_000_000_000i64,
        "agentSession": {
            "id": SESSION,
            "issue": {"id": ISSUE, "identifier": "ENG-7", "title": "Fix it", "teamId": TEAM},
        },
    })
}

// =========================================================================
// Assertions
// =========================================================================

#[track_caller]
fn kinds_of(outcome: Result<Outcome, Rejection>) -> Vec<&'static str> {
    match outcome {
        Ok(Outcome::Admitted { kinds, .. }) => kinds,
        other => panic!("expected an admission, got {other:?}"),
    }
}

#[track_caller]
fn reason_of(outcome: Result<Outcome, Rejection>) -> &'static str {
    match outcome {
        Ok(Outcome::Ignored { reason, .. }) => reason,
        other => panic!("expected a drop, got {other:?}"),
    }
}

// =========================================================================
// The seam itself
// =========================================================================

/// The criterion this whole ticket rests on: every ingress decision is
/// reachable with an `AppState`, a signature and a body, and nothing else.
#[sqlx::test]
async fn receive_admits_a_signed_delivery_without_an_http_request(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool).await;

    let outcome = deliver(&state, &comment_envelope("create", "c-1")).await;

    assert!(
        matches!(&outcome, Ok(Outcome::Admitted { event_type, .. }) if event_type == "Comment")
    );
    assert_eq!(kinds_of(outcome), vec![delivery::COMMENT]);
    assert_eq!(admitted_count(&pool).await, 1);
    assert!(
        stub.graphql_operations().is_empty(),
        "a comment costs no Linear call before the acknowledgment"
    );
}

/// Each event type fans into its own kind, and the identity every request
/// carries is the one that collapses a retry.
#[sqlx::test]
async fn each_event_type_admits_its_own_kind_and_a_redelivery_admits_nothing(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool).await;

    assert_eq!(
        kinds_of(deliver(&state, &issue_envelope(true)).await),
        vec![delivery::ISSUE_UPDATE]
    );

    let mut prompted = session_envelope("prompted");
    prompted["agentActivity"] = serde_json::json!({"id": "act-1", "body": "do the thing"});
    assert_eq!(
        kinds_of(deliver(&state, &prompted).await),
        vec![delivery::SESSION_PROMPT]
    );

    let notification = serde_json::json!({
        "action": "issueCommentMention",
        "type": "AppUserNotification",
        "organizationId": ORG,
        "webhookTimestamp": 1_700_000_000_000i64,
        "notification": {"id": "n-1", "type": "issueCommentMention"},
    });
    assert_eq!(
        kinds_of(deliver(&state, &notification).await),
        vec![delivery::NOTIFICATION]
    );

    assert_eq!(admitted_count(&pool).await, 3);

    // The same three envelopes again: Linear's retry, and the identity it is
    // recognised by lives in `adapter_delivery`, not in this process.
    for envelope in [issue_envelope(true), prompted, notification] {
        let outcome = deliver(&state, &envelope).await;
        assert!(
            matches!(outcome, Ok(Outcome::Admitted { .. })),
            "a redelivery is still an admission, it just commits nothing new"
        );
    }
    assert_eq!(
        admitted_count(&pool).await,
        3,
        "a redelivery must not write a second record"
    );
}

/// A created session: the heartbeat is posted before the acknowledgment, and
/// the provisioning behind it is admitted rather than run.
#[sqlx::test]
async fn a_created_session_posts_its_heartbeat_then_admits_provisioning(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool).await;
    seed_workspace_token(&pool).await;
    seed_rule(&state, &pool).await;

    let outcome = deliver(&state, &session_envelope("created")).await;

    assert_eq!(kinds_of(outcome), vec![delivery::SESSION_PROVISION]);
    assert_eq!(
        stub.graphql_operations(),
        vec!["agentActivityCreate"],
        "exactly one call on the response path — the panel heartbeat"
    );
    assert_eq!(admitted_count(&pool).await, 1);
}

// =========================================================================
// The seven reasons, one real delivery each
// =========================================================================

#[sqlx::test]
async fn an_unsigned_delivery_is_no_signature(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool).await;

    let body = serde_json::to_vec(&comment_envelope("create", "c-1")).expect("serialize");
    assert_eq!(
        reason_of(deliver_signed_bytes(&state, body, false).await),
        reason::NO_SIGNATURE
    );
    assert_eq!(admitted_count(&pool).await, 0);
}

/// Both spellings of "we do not know this workspace": an organization with no
/// connection row, and an envelope that names no organization at all.
#[sqlx::test]
async fn an_unknown_or_absent_organization_is_no_connection(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    // Deliberately no connection seeded.

    assert_eq!(
        reason_of(deliver(&state, &comment_envelope("create", "c-1")).await),
        reason::NO_CONNECTION
    );

    let mut anonymous = comment_envelope("create", "c-2");
    anonymous
        .as_object_mut()
        .expect("object")
        .remove("organizationId");
    assert_eq!(
        reason_of(deliver(&state, &anonymous).await),
        reason::NO_CONNECTION
    );
}

#[sqlx::test]
async fn an_event_type_this_adapter_does_not_handle_is_unsupported_event(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool).await;

    let envelope = serde_json::json!({
        "action": "create",
        "type": "Project",
        "organizationId": ORG,
        "webhookTimestamp": 1_700_000_000_000i64,
        "data": {"id": "proj-1"},
    });
    assert_eq!(
        reason_of(deliver(&state, &envelope).await),
        reason::UNSUPPORTED_EVENT
    );
}

#[sqlx::test]
async fn an_action_this_adapter_does_not_act_on_is_unsupported_action(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool).await;

    let mut created_issue = issue_envelope(true);
    created_issue["action"] = serde_json::json!("create");

    for envelope in [
        comment_envelope("update", "c-1"),
        created_issue,
        session_envelope("archived"),
    ] {
        assert_eq!(
            reason_of(deliver(&state, &envelope).await),
            reason::UNSUPPORTED_ACTION
        );
    }
    assert_eq!(admitted_count(&pool).await, 0);
}

#[sqlx::test]
async fn an_absent_or_unparseable_inner_payload_is_malformed_payload(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool).await;

    let mut no_data = comment_envelope("create", "c-1");
    no_data.as_object_mut().expect("object").remove("data");

    let mut wrong_shape = comment_envelope("create", "c-1");
    wrong_shape["data"] = serde_json::json!({"id": 17});

    let mut sessionless = session_envelope("created");
    sessionless
        .as_object_mut()
        .expect("object")
        .remove("agentSession");

    for envelope in [no_data, wrong_shape, sessionless] {
        assert_eq!(
            reason_of(deliver(&state, &envelope).await),
            reason::MALFORMED_PAYLOAD
        );
    }
}

/// The interim answer to the plan's Open question 2: nothing durable records
/// that Linear asked and we declined, but the metric now says so — and the
/// heartbeat is still posted, because the panel is already open.
#[sqlx::test]
async fn a_created_session_with_no_active_rule_is_no_subscription(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool).await;
    seed_workspace_token(&pool).await;
    // Deliberately no rule seeded.

    assert_eq!(
        reason_of(deliver(&state, &session_envelope("created")).await),
        reason::NO_SUBSCRIPTION
    );
    assert_eq!(
        stub.graphql_operations(),
        vec!["agentActivityCreate"],
        "the heartbeat survives the drop"
    );
    assert_eq!(admitted_count(&pool).await, 0);
}

#[sqlx::test]
async fn a_prompt_carrying_nothing_to_forward_is_empty_body(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool).await;

    let mut empty = session_envelope("prompted");
    empty["agentActivity"] = serde_json::json!({"id": "act-1", "body": ""});

    for envelope in [empty, session_envelope("prompted")] {
        assert_eq!(
            reason_of(deliver(&state, &envelope).await),
            reason::EMPTY_BODY
        );
    }
    assert_eq!(admitted_count(&pool).await, 0);
}

// =========================================================================
// The three refusals, and the event type each carries
// =========================================================================

/// The delivery carries work but no field that tells that work apart from the
/// next update to the same issue, so it is refused rather than admitted under a
/// colliding identity.
#[sqlx::test]
async fn an_issue_update_with_no_timestamp_is_unidentifiable(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool).await;

    let rejection = deliver(&state, &issue_envelope(false))
        .await
        .expect_err("refused");

    assert!(
        matches!(
            rejection.error,
            IngressError::Unidentifiable("webhookTimestamp")
        ),
        "got {:?}",
        rejection.error
    );
    assert_eq!(rejection.event_type, "Issue");
    assert_eq!(admitted_count(&pool).await, 0);
}

#[sqlx::test]
async fn a_bad_signature_is_an_ingress_rejection_naming_its_event_type(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool).await;

    let body = serde_json::to_vec(&comment_envelope("create", "c-1")).expect("serialize");
    let provider = state
        .adapters
        .as_ref()
        .expect("adapters registered")
        .linear()
        .expect("Linear adapter registered");
    let rejection = receive(&state, provider, "00ff", &body)
        .await
        .expect_err("refused");

    assert!(matches!(rejection.error, IngressError::Ingress(_)));
    assert_eq!(rejection.event_type, "Comment");
}

/// The one refusal with no event type to carry, because the body never parsed.
#[sqlx::test]
async fn an_unparseable_body_is_an_ingress_rejection_with_no_event_type(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool).await;

    let rejection = deliver_signed_bytes(&state, b"not json".to_vec(), true)
        .await
        .expect_err("refused");

    assert!(matches!(rejection.error, IngressError::Ingress(_)));
    assert_eq!(rejection.event_type, "");
}

/// The one failure that must not be acknowledged. Nothing was written, so the
/// route answers non-2xx and asks Linear to redeliver — and the row it meters
/// names the kind of delivery that was lost.
#[sqlx::test]
async fn a_failed_admission_is_not_admitted_and_still_names_its_event_type(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool).await;

    // The durable write is the thing that has to fail, and it is the only thing
    // that may: every guard above it has already passed.
    sqlx::query("DROP TABLE adapter_delivery")
        .execute(&pool)
        .await
        .expect("drop the admission table");

    let rejection = deliver(&state, &comment_envelope("create", "c-1"))
        .await
        .expect_err("refused");

    assert!(
        matches!(rejection.error, IngressError::NotAdmitted(_)),
        "got {:?}",
        rejection.error
    );
    assert_eq!(rejection.event_type, "Comment");
}
