//! What the database decides about a pairing code.
//!
//! The HTTP shape of the exchange is asserted against mocks in
//! `crates/t3code/src/lib.rs`; the parts that are only true of Postgres are
//! here. All three are the same property from different sides: a code is worth
//! exactly one credential, once, for five minutes — and the statement that
//! claims it is the only thing standing between a leaked code and a second
//! device holding the account.

use std::sync::Arc;

use chrono::{Duration, Utc};
use moatless_types::user::{UserRole, UserStatus};
use sqlx::PgPool;
use uuid::Uuid;

use super::{PairingCodeDao, PgPairingCodeDao};
use crate::auth::api_key::validate_key;
use crate::auth::api_keys_dao::{ApiKeyDao, PgApiKeyDao};
use crate::auth::pairing::{
    PAIRED_DEVICE_KEY_TTL, PAIRING_CODE_TTL, PairingClient, PairingCode, PairingError,
};
use crate::auth::users::User;
use crate::auth::users_dao::{PgUserDao, UserDao};

const USER_1: &str = "11111111-1111-1111-1111-111111111111";

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
            email: Some("pairing-user@example.test".into()),
            name: None,
            avatar_url: None,
            provider: "local".into(),
            provider_user_id: Some("local:pairing-user".into()),
            role: UserRole::User,
            created_at: Utc::now(),
            last_login_at: Utc::now(),
            settings: serde_json::json!({}),
            is_bot: false,
            status: UserStatus::Active,
        })
        .await
        .unwrap();
    id
}

#[sqlx::test]
async fn a_redeemed_code_leaves_the_device_holding_a_working_key(pool: PgPool) {
    let user_id = seed_user(&pool).await;
    let pairing = crate::compose::device_pairing(pool.clone());

    let issued = pairing
        .issue_code(&user_id.to_string(), Some("Bedside phone"))
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
    let pairing = crate::compose::device_pairing(pool.clone());
    let issued = pairing
        .issue_code(&user_id.to_string(), None)
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
            code_hash: crate::auth::pairing::hash_key(code),
            user_id: user_id.to_string(),
            label: None,
            created_at: Utc::now() - Duration::hours(1),
            expires_at: Utc::now() - Duration::minutes(55),
        })
        .await
        .unwrap();

    let redeemed = crate::compose::device_pairing(pool.clone())
        .redeem_code(code, &phone(), scopes())
        .await;
    assert!(matches!(redeemed, Err(PairingError::InvalidCode)));
}

#[sqlx::test]
async fn a_lower_case_code_is_the_same_code(pool: PgPool) {
    let user_id = seed_user(&pool).await;
    let pairing = crate::compose::device_pairing(pool.clone());
    let issued = pairing
        .issue_code(&user_id.to_string(), None)
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

#[sqlx::test]
async fn a_paired_key_slides_its_expiry_and_a_bare_stamp_does_not(pool: PgPool) {
    let user_id = seed_user(&pool).await;
    let pairing = crate::compose::device_pairing(pool.clone());
    let issued = pairing
        .issue_code(&user_id.to_string(), None)
        .await
        .unwrap();
    let device = pairing
        .redeem_code(&issued.code, &phone(), scopes())
        .await
        .unwrap();

    let api_keys = PgApiKeyDao(pool.clone());
    let row = api_keys
        .get_api_key_by_id(&device.key_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.idle_ttl_secs,
        Some(PAIRED_DEVICE_KEY_TTL.num_seconds()),
        "pairing mints a key that renews on use"
    );

    let renewed = Utc::now() + Duration::days(200);
    api_keys
        .update_last_used(&device.key_id, Some(renewed))
        .await
        .unwrap();
    let after_renewal = api_keys
        .get_api_key_by_id(&device.key_id)
        .await
        .unwrap()
        .unwrap()
        .expires_at
        .unwrap();
    // Postgres keeps microseconds, so the round trip can drop nanoseconds.
    assert!((after_renewal - renewed).num_milliseconds().abs() < 1);

    api_keys
        .update_last_used(&device.key_id, None)
        .await
        .unwrap();
    let after_stamp = api_keys
        .get_api_key_by_id(&device.key_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_stamp.expires_at, Some(after_renewal));
    assert!(after_stamp.last_used_at.is_some());
}

#[sqlx::test]
async fn only_unredeemed_codes_are_listed_and_only_their_owner_revokes_one(pool: PgPool) {
    let user_id = seed_user(&pool).await.to_string();
    let pairing = crate::compose::device_pairing(pool.clone());
    let redeemed = pairing.issue_code(&user_id, None).await.unwrap();
    pairing
        .redeem_code(&redeemed.code, &phone(), scopes())
        .await
        .unwrap();
    let pending = pairing.issue_code(&user_id, Some("Tablet")).await.unwrap();

    let listed = pairing.pending_codes(&user_id).await.unwrap();
    assert_eq!(listed.len(), 1, "a redeemed code is no longer a link");
    assert_eq!(listed[0].id, pending.id);
    assert_eq!(listed[0].label.as_deref(), Some("Tablet"));

    assert!(!pairing.revoke_code(&user_id, &redeemed.id).await.unwrap());
    assert!(
        !pairing
            .revoke_code("22222222-2222-2222-2222-222222222222", &pending.id)
            .await
            .unwrap(),
        "another account cannot revoke the link"
    );
    assert!(pairing.revoke_code(&user_id, &pending.id).await.unwrap());
    assert!(pairing.pending_codes(&user_id).await.unwrap().is_empty());
}

#[sqlx::test]
async fn unpairing_deletes_paired_keys_and_leaves_ordinary_ones(pool: PgPool) {
    let user_id = seed_user(&pool).await.to_string();
    let pairing = crate::compose::device_pairing(pool.clone());
    let api_keys = PgApiKeyDao(pool.clone());
    let ordinary = crate::auth::api_key::generate_key(
        &api_keys,
        &user_id,
        "CI token",
        None,
        Some(Utc::now() + Duration::days(30)),
        None,
    )
    .await
    .unwrap();
    let mut paired = Vec::new();
    for _ in 0..2 {
        let issued = pairing.issue_code(&user_id, None).await.unwrap();
        let device = pairing
            .redeem_code(&issued.code, &phone(), scopes())
            .await
            .unwrap();
        paired.push(device.key_id);
    }

    let devices = pairing.devices(&user_id).await.unwrap();
    assert_eq!(devices.len(), 2, "an ordinary API key is not a device");
    assert!(devices.iter().all(|device| paired.contains(&device.id)));
    assert_eq!(devices[0].name, "T3 Code Mobile (iOS)");

    assert!(!pairing.revoke_device(&user_id, &ordinary.id).await.unwrap());
    assert!(
        !pairing
            .revoke_device("22222222-2222-2222-2222-222222222222", &paired[0])
            .await
            .unwrap(),
        "another account cannot unpair the device"
    );
    assert!(pairing.revoke_device(&user_id, &paired[0]).await.unwrap());
    assert_eq!(pairing.revoke_all_devices(&user_id).await.unwrap(), 1);
    assert!(pairing.devices(&user_id).await.unwrap().is_empty());

    let left = api_keys.list_api_keys_by_user(&user_id).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, ordinary.id);
}
