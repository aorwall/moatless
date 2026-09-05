#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! What the database decides about a pairing code.
//!
//! The HTTP shape of the exchange is asserted against mocks in
//! `crates/t3code/src/lib.rs`; the parts that are only true of Postgres are
//! here. All three are the same property from different sides: a code is worth
//! exactly one credential, once, for five minutes — and the statement that
//! claims it is the only thing standing between a leaked code and a second
//! device holding the account.

use crate::common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use moatless_identity::auth::api_key::validate_key;
use moatless_identity::auth::api_keys::ApiKeyDao;
use moatless_identity::auth::api_keys::PgApiKeyDao;
use moatless_identity::auth::pairing::DevicePairing;
use moatless_identity::auth::pairing::PAIRED_DEVICE_KEY_TTL;
use moatless_identity::auth::pairing::PAIRING_CODE_TTL;
use moatless_identity::auth::pairing::PairingClient;
use moatless_identity::auth::pairing::PairingCode;
use moatless_identity::auth::pairing::PairingCodeDao;
use moatless_identity::auth::pairing::PairingError;
use moatless_identity::auth::pairing::PgPairingApiKeyIssuer;
use moatless_identity::auth::pairing::PgPairingCodeDao;
use moatless_identity::auth::users::PgUserDao;
use moatless_identity::auth::users::User;
use moatless_identity::auth::users::UserDao;
use moatless_types::user::UserRole;
use sqlx::PgPool;
use uuid::Uuid;

use common::USER_1;

fn pairing(pool: &PgPool) -> DevicePairing {
    DevicePairing::new(
        Arc::new(PgPairingCodeDao(pool.clone())),
        Arc::new(PgPairingApiKeyIssuer(Arc::new(PgApiKeyDao(pool.clone())))),
    )
}

fn phone() -> PairingClient {
    PairingClient {
        label: Some("T3 Code Mobile".into()),
        os: Some("iOS".into()),
    }
}

fn scopes() -> Vec<String> {
    vec!["orchestration:read".into(), "relay:read".into()]
}

/// The account the code is minted for. `validate_key` reads it back to report
/// who the key belongs to, so a key with no user behind it proves nothing.
async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::parse_str(USER_1).unwrap();
    PgUserDao(pool.clone())
        .upsert_user(&User {
            id,
            login: "pairing-user".into(),
            email: None,
            name: None,
            avatar_url: None,
            provider: "local".into(),
            provider_user_id: Some("local:pairing-user".into()),
            role: UserRole::User,
            created_at: Utc::now(),
            last_login_at: Utc::now(),
            settings: serde_json::json!({}),
            is_bot: false,
        })
        .await
        .unwrap();
    id
}

#[sqlx::test]
async fn a_redeemed_code_leaves_the_device_holding_a_working_key(pool: PgPool) {
    let user_id = seed_user(&pool).await;
    let pairing = pairing(&pool);

    let issued = pairing
        .issue_code(&user_id.to_string(), None, Some("Bedside phone"))
        .await
        .unwrap();
    assert!(
        issued.expires_at > Utc::now() && issued.expires_at <= Utc::now() + PAIRING_CODE_TTL,
        "a fresh code is alive for the window it advertises"
    );

    let device = pairing
        .redeem_code(&issued.code, &phone(), scopes())
        .await
        .unwrap();

    let api_keys: Arc<dyn ApiKeyDao> = Arc::new(PgApiKeyDao(pool.clone()));
    let users = PgUserDao(pool.clone());
    let payload = validate_key(api_keys, &users, &device.token)
        .await
        .unwrap()
        .expect("the token a paired device holds authenticates it");
    assert_eq!(payload.user_id, user_id.to_string());
    assert_eq!(payload.user_name.as_deref(), Some("pairing-user"));

    // The key is bounded: a phone that is lost stops being the account.
    let expires_at = payload.expires_at.expect("a paired key expires");
    assert!(expires_at > Utc::now() + PAIRED_DEVICE_KEY_TTL - Duration::minutes(1));
    assert!(expires_at <= Utc::now() + PAIRED_DEVICE_KEY_TTL);

    // And it is recognizable in the list its owner revokes from.
    let listed = PgApiKeyDao(pool.clone())
        .list_api_keys_by_user(&user_id.to_string())
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Bedside phone (iOS)");
}

#[sqlx::test]
async fn a_code_is_worth_exactly_one_device(pool: PgPool) {
    let user_id = seed_user(&pool).await;
    let pairing = pairing(&pool);
    let issued = pairing
        .issue_code(&user_id.to_string(), None, None)
        .await
        .unwrap();

    pairing
        .redeem_code(&issued.code, &phone(), scopes())
        .await
        .unwrap();

    // The `UPDATE ... WHERE consumed_at IS NULL` matches nothing the second
    // time, which is what makes a replayed code — a photographed screen, a
    // pasted chat message — worth nothing.
    let replayed = pairing.redeem_code(&issued.code, &phone(), scopes()).await;
    assert!(matches!(replayed, Err(PairingError::InvalidCode)));

    assert_eq!(
        PgApiKeyDao(pool.clone())
            .list_api_keys_by_user(&user_id.to_string())
            .await
            .unwrap()
            .len(),
        1,
        "the refused replay minted nothing"
    );
}

#[sqlx::test]
async fn a_code_that_ran_out_of_time_is_refused(pool: PgPool) {
    let user_id = seed_user(&pool).await;
    let codes = PgPairingCodeDao(pool.clone());

    // Written straight through the DAO: the service will not mint a code that
    // is already dead, and the clock cannot be moved.
    let code = "ABCDEFGHJKLM";
    codes
        .insert_pairing_code(&PairingCode {
            id: "expired".into(),
            code_hash: moatless_identity::auth::pairing::hash_key(code),
            user_id: user_id.to_string(),
            group_id: None,
            label: None,
            created_at: Utc::now() - Duration::hours(1),
            expires_at: Utc::now() - Duration::minutes(55),
        })
        .await
        .unwrap();

    let redeemed = pairing(&pool).redeem_code(code, &phone(), scopes()).await;
    assert!(matches!(redeemed, Err(PairingError::InvalidCode)));
}

#[sqlx::test]
async fn a_lower_case_code_is_the_same_code(pool: PgPool) {
    let user_id = seed_user(&pool).await;
    let pairing = pairing(&pool);
    let issued = pairing
        .issue_code(&user_id.to_string(), None, None)
        .await
        .unwrap();

    // What a phone keyboard does to a code someone reads off a laptop, and the
    // spacing they may type it back in.
    let typed = format!(
        "{}-{}",
        issued.code[..6].to_lowercase(),
        issued.code[6..].to_lowercase()
    );
    assert!(
        pairing
            .redeem_code(&typed, &phone(), scopes())
            .await
            .is_ok()
    );
}

#[sqlx::test]
async fn pruning_clears_finished_codes_and_leaves_live_ones(pool: PgPool) {
    let user_id = seed_user(&pool).await;
    let codes = PgPairingCodeDao(pool.clone());
    for (id, expires_at) in [
        ("old", Utc::now() - Duration::days(2)),
        ("live", Utc::now() + Duration::minutes(5)),
    ] {
        codes
            .insert_pairing_code(&PairingCode {
                id: id.into(),
                code_hash: format!("hash-{id}"),
                user_id: user_id.to_string(),
                group_id: None,
                label: None,
                created_at: Utc::now() - Duration::days(2),
                expires_at,
            })
            .await
            .unwrap();
    }

    let deleted = codes
        .delete_finished_pairing_codes(Utc::now() - Duration::days(1))
        .await
        .unwrap();
    assert_eq!(deleted, 1);
    assert!(
        codes
            .consume_pairing_code("hash-live", Utc::now())
            .await
            .unwrap()
            .is_some()
    );
}
