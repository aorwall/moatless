//! Team CRUD, membership, and credentials integration tests.
//!
//! Mirrors: tests/api/chains/auth-team-crud.spec.ts
//! Spec IDs: AUTH-01 through AUTH-10

use integration_tests::{client_as_user, raw_api, raw_client_as_user, seed_test_users};
use moatless_api_schemas::team::{
    AddTeamMemberRequest, CreateTeamRequest, UpdateTeamRequest, UpsertTeamCredentialRequest,
};
use pretty_assertions::assert_eq;

fn timestamp_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}

// =============================================================================
// Chain 1: Team CRUD (AUTH-01 through AUTH-04.1)
// =============================================================================

#[tokio::test]
async fn team_crud_chain() {
    seed_test_users().await;
    let alice = client_as_user("alice-test", "alice", "user");
    let suffix = timestamp_suffix();

    // AUTH-01: Create a team
    let team = alice
        .api()
        .create_team(&CreateTeamRequest {
            name: "Engineering".into(),
            slug: format!("engineering-{suffix}"),
            description: None,
        })
        .await
        .unwrap();
    assert!(!team.id.is_empty());
    assert_eq!(team.name, "Engineering");
    assert_eq!(team.slug, format!("engineering-{suffix}"));

    let team_id = team.id;

    // AUTH-01.1: Team appears in listing
    let teams = alice.api().list_teams().await.unwrap();
    assert!(teams.iter().any(|t| t.id == team_id));

    // AUTH-02: Get team by ID
    let got = alice.api().get_team(&team_id).await.unwrap();
    assert_eq!(got.name, "Engineering");
    assert_eq!(got.slug, format!("engineering-{suffix}"));

    // AUTH-03: Update team
    let updated = alice
        .api()
        .update_team(
            &team_id,
            &UpdateTeamRequest {
                name: "Platform Engineering".into(),
                description: Some("Platform team".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "Platform Engineering");
    assert_eq!(updated.description.as_deref(), Some("Platform team"));
    assert_eq!(updated.slug, format!("engineering-{suffix}"));

    // AUTH-04: Delete team
    alice.api().delete_team(&team_id).await.unwrap();

    // AUTH-04.1: Team removed from listing
    let teams = alice.api().list_teams().await.unwrap();
    assert!(!teams.iter().any(|t| t.id == team_id));
}

// =============================================================================
// Chain 2: Team Membership (AUTH-05 through AUTH-07.1)
// =============================================================================

#[tokio::test]
async fn team_membership_chain() {
    seed_test_users().await;
    let alice = client_as_user("alice-test", "alice", "user");
    let suffix = timestamp_suffix();

    // Setup: Alice creates a team
    let team = alice
        .api()
        .create_team(&CreateTeamRequest {
            name: "Membership Team".into(),
            slug: format!("membership-{suffix}"),
            description: None,
        })
        .await
        .unwrap();
    let team_id = team.id;

    // AUTH-05: Add member to team
    alice
        .api()
        .add_member(
            &team_id,
            &AddTeamMemberRequest {
                user_id: "bob-test".into(),
                role: Some("member".into()),
            },
        )
        .await
        .unwrap();

    // AUTH-05.1: Member appears in team member listing
    let members = alice.api().list_members(&team_id).await.unwrap();
    let bob = members.iter().find(|m| m.user_id == "bob-test");
    assert!(bob.is_some());
    assert_eq!(bob.unwrap().role, "member");

    // AUTH-07: Remove member from team
    alice.api().remove_member(&team_id, "bob-test").await.unwrap();

    // AUTH-07.1: Removed member no longer in listing
    let members = alice.api().list_members(&team_id).await.unwrap();
    assert!(!members.iter().any(|m| m.user_id == "bob-test"));

    // Cleanup
    let _ = alice.api().delete_team(&team_id).await;
}

// =============================================================================
// Chain 3: Team Credentials (AUTH-08 through AUTH-08.2.1)
// =============================================================================

#[tokio::test]
async fn team_credentials_chain() {
    seed_test_users().await;
    let alice = client_as_user("alice-test", "alice", "user");
    let suffix = timestamp_suffix();

    // Setup: Alice creates a team
    let team = alice
        .api()
        .create_team(&CreateTeamRequest {
            name: "Credentials Team".into(),
            slug: format!("credentials-{suffix}"),
            description: None,
        })
        .await
        .unwrap();
    let team_id = team.id;

    // AUTH-08: Upsert team credential
    alice
        .api()
        .upsert_credential(
            &team_id,
            "github",
            &UpsertTeamCredentialRequest {
                access_token: "ghp_test_token_12345".into(),
                metadata: None,
                refresh_token: None,
                token_type: None,
            },
        )
        .await
        .unwrap();

    // AUTH-08.1: Credential appears in listing
    let creds = alice
        .api()
        .list_credentials(&team_id)
        .await
        .unwrap();
    let github = creds.iter().find(|c| c.provider == "github");
    assert!(github.is_some());
    // Access token must NOT be exposed in listing
    let cred_json = serde_json::to_string(&github.unwrap()).unwrap();
    assert!(!cred_json.contains("ghp_test_token"));

    // AUTH-08.2: Delete team credential
    alice.api().delete_credential(&team_id, "github").await.unwrap();

    // AUTH-08.2.1: Credential removed from listing
    let creds = alice
        .api()
        .list_credentials(&team_id)
        .await
        .unwrap();
    assert!(!creds.iter().any(|c| c.provider == "github"));

    // Cleanup
    let _ = alice.api().delete_team(&team_id).await;
}

// =============================================================================
// Standalone: AUTH-09 (Duplicate slug → 409)
// =============================================================================

#[tokio::test]
async fn team_duplicate_slug() {
    seed_test_users().await;
    let alice = client_as_user("alice-test", "alice", "user");
    let alice_raw = raw_client_as_user("alice-test", "alice", "user");
    let suffix = timestamp_suffix();
    let slug = format!("dup-slug-{suffix}");

    // Create first team
    let team = alice
        .api()
        .create_team(&CreateTeamRequest {
            name: "First".into(),
            slug: slug.clone(),
            description: None,
        })
        .await
        .unwrap();

    // Attempt duplicate
    let resp = raw_api::create_team_raw(
        &alice_raw,
        &serde_json::json!({ "name": "Second", "slug": slug }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 409);

    // Cleanup
    let _ = alice.api().delete_team(&team.id).await;
}

// =============================================================================
// Standalone: AUTH-10 (Non-member credential upsert → 403)
// =============================================================================

#[tokio::test]
async fn nonmember_credential_access() {
    seed_test_users().await;
    let alice = client_as_user("alice-test", "alice", "user");
    let bob_raw = raw_client_as_user("bob-test", "bob", "user");
    let suffix = timestamp_suffix();

    // Alice creates a team (bob is not a member)
    let team = alice
        .api()
        .create_team(&CreateTeamRequest {
            name: "Private Team".into(),
            slug: format!("private-{suffix}"),
            description: None,
        })
        .await
        .unwrap();

    // Bob tries to upsert credential — should be forbidden
    let resp = raw_api::upsert_team_credential(
        &bob_raw,
        &team.id,
        "github",
        &serde_json::json!({ "accessToken": "ghp_unauthorized" }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 403);

    // Cleanup
    let _ = alice.api().delete_team(&team.id).await;
}
