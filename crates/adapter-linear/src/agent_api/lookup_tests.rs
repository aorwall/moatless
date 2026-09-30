//! The filter each [`LookupKind`] sends, and which of the answered nodes a name
//! resolves to.
//!
//! Every filter case here was run against the live API before it was written
//! down: Linear rejects an unknown filter field outright, and a stub would
//! happily accept one this crate invented.

use moatless_adapter_model::test_support::ProviderStub;
use moatless_ports::identity::MockAppCredentials;
use serde_json::json;

use super::*;

const TEAM: &str = "3fa49663-046d-46db-9566-584753ff4b23";
const SP_IN_REVIEW: &str = "96aece86-e8ab-40fb-a33b-fd9fabc7d679";
const OPS_IN_REVIEW: &str = "4f1278fb-0751-4b1c-affc-5d8acff385e5";

/// The document and filter one lookup put on the wire.
async fn request_for(kind: LookupKind, team_id: Option<&str>) -> (String, serde_json::Value) {
    let stub = ProviderStub::start(|_| json!({ "data": {} })).await;
    let mut tokens = MockAppCredentials::new();
    tokens.expect_app_token().returning(|_| {
        Ok(Some(AppToken {
            access_token: "lin_test".into(),
            refresh_token: None,
            token_type: None,
            expires_at: Some(Utc::now() + chrono::Duration::days(1)),
            metadata: None,
        }))
    });
    let api = LinearAgentApi::with_base_url(
        crate::agent_api::http_client(moatless_core::config::HttpConfig::default().linear),
        Arc::new(tokens),
        "test-org".to_string(),
        None,
        None,
        stub.base_url(),
    );

    api.lookup(kind, "Canceled", team_id)
        .await
        .expect("the stub answers");

    let body: serde_json::Value = serde_json::from_str(&stub.requests()[0].body)
        .expect("the recorded body is the JSON we sent");
    (
        body["query"].as_str().expect("query is a string").into(),
        body["variables"]["filter"].clone(),
    )
}

#[tokio::test]
async fn a_state_lookup_narrows_to_the_team() {
    let (query, filter) = request_for(LookupKind::State, Some(TEAM)).await;
    assert!(
        query.contains("query($filter: WorkflowStateFilter)"),
        "{query}"
    );
    assert_eq!(
        filter,
        json!({
            "name": { "eqIgnoreCase": "Canceled" },
            "team": { "id": { "eq": TEAM } }
        })
    );
}

/// A workspace label has no team and stays usable on every issue, so the
/// team clause has to admit a null team alongside the match.
#[tokio::test]
async fn a_label_lookup_keeps_workspace_labels_in_reach() {
    let (query, filter) = request_for(LookupKind::Label, Some(TEAM)).await;
    assert!(
        query.contains("query($filter: IssueLabelFilter)"),
        "{query}"
    );
    assert_eq!(
        filter,
        json!({
            "name": { "eqIgnoreCase": "Canceled" },
            "or": [
                { "team": { "id": { "eq": TEAM } } },
                { "team": { "null": true } }
            ]
        })
    );
}

/// A project is shared with teams rather than owned by one, so
/// `ProjectFilter` carries no `team` field to filter on.
#[tokio::test]
async fn a_project_lookup_narrows_through_accessible_teams() {
    let (query, filter) = request_for(LookupKind::Project, Some(TEAM)).await;
    assert!(query.contains("query($filter: ProjectFilter)"), "{query}");
    assert_eq!(
        filter,
        json!({
            "name": { "eqIgnoreCase": "Canceled" },
            "accessibleTeams": { "some": { "id": { "eq": TEAM } } }
        })
    );
}

/// Users and teams are workspace-global: a team id passed anyway is dropped
/// rather than turned into a filter field neither collection has.
#[tokio::test]
async fn users_and_teams_ignore_a_team_scope() {
    for kind in [LookupKind::User, LookupKind::Team] {
        let (_, filter) = request_for(kind, Some(TEAM)).await;
        assert_eq!(filter, json!({ "name": { "eqIgnoreCase": "Canceled" } }));
    }
}

#[tokio::test]
async fn an_unscoped_lookup_filters_on_the_name_alone() {
    let (_, filter) = request_for(LookupKind::State, None).await;
    assert_eq!(filter, json!({ "name": { "eqIgnoreCase": "Canceled" } }));
}

/// One `workflowStates` node, as [`select_lookup_match`] receives it. The id it
/// picks from these is the id a write puts on the issue.
fn state(id: &str, name: &str, team_key: &str) -> serde_json::Value {
    json!({
        "id": id,
        "name": name,
        "type": "started",
        "team": { "id": TEAM, "key": team_key },
    })
}

#[test]
fn one_match_resolves() {
    let found = select_lookup_match(
        LookupKind::State,
        "In Review",
        Some(TEAM),
        vec![state(SP_IN_REVIEW, "In Review", "SP")],
    )
    .expect("one match is not ambiguous")
    .expect("the match resolves");
    assert_eq!(found.id, SP_IN_REVIEW);
    assert_eq!(found.name, "In Review");
}

#[test]
fn no_match_resolves_to_nothing() {
    let found = select_lookup_match(LookupKind::State, "In Review", Some(TEAM), vec![])
        .expect("an empty answer is not ambiguous");
    assert!(found.is_none());
}

/// The failure this guard exists for: `In Review` naming a state on two
/// teams must not silently write whichever Linear listed first.
#[test]
fn two_teams_sharing_a_state_name_is_refused() {
    let err = select_lookup_match(
        LookupKind::State,
        "In Review",
        None,
        vec![
            state(OPS_IN_REVIEW, "In Review", "OPS"),
            state(SP_IN_REVIEW, "In Review", "SP"),
        ],
    )
    .expect_err("two teams' states must not resolve to one");
    let message = err.to_string();
    assert!(message.contains("ambiguous"), "{message}");
    assert!(message.contains("team OPS"), "{message}");
    assert!(message.contains("team SP"), "{message}");
}

/// Linear filters on `eqIgnoreCase`, so this drops nothing today. It is the
/// guard that keeps a widened filter from resolving `In Review` to
/// `Canceled`.
#[test]
fn a_differently_named_state_is_not_a_match() {
    let found = select_lookup_match(
        LookupKind::State,
        "In Review",
        Some(TEAM),
        vec![state(
            "23123317-0000-0000-0000-000000000000",
            "Canceled",
            "SP",
        )],
    )
    .expect("one node is not ambiguous");
    assert!(found.is_none(), "a state named Canceled is not `In Review`");
}

#[test]
fn case_differences_still_match() {
    let found = select_lookup_match(
        LookupKind::State,
        "in review",
        Some(TEAM),
        vec![state(SP_IN_REVIEW, "In Review", "SP")],
    )
    .expect("one match is not ambiguous")
    .expect("case-insensitive match resolves");
    assert_eq!(found.id, SP_IN_REVIEW);
}

/// The team clause admits workspace labels alongside the team's own, so the
/// pair must resolve rather than read as two objects with one name.
#[test]
fn a_team_label_shadows_the_workspace_label() {
    let workspace = json!({ "id": "label-workspace", "name": "moatless", "team": null });
    let team = json!({
        "id": "label-team",
        "name": "moatless",
        "team": { "id": TEAM, "key": "SP" },
    });
    let found = select_lookup_match(
        LookupKind::Label,
        "moatless",
        Some(TEAM),
        vec![workspace, team],
    )
    .expect("the team label wins")
    .expect("the label resolves");
    assert_eq!(found.id, "label-team");
}

/// Without a team there is nothing to prefer, so the same pair is refused.
#[test]
fn an_unscoped_label_name_matching_twice_is_refused() {
    let err = select_lookup_match(
        LookupKind::Label,
        "moatless",
        None,
        vec![
            json!({ "id": "label-workspace", "name": "moatless", "team": null }),
            json!({
                "id": "label-team",
                "name": "moatless",
                "team": { "id": TEAM, "key": "SP" },
            }),
        ],
    )
    .expect_err("two labels must not resolve to one");
    assert!(err.to_string().contains("ambiguous"), "{err}");
}

/// Linear pages can repeat a node; the same id twice is one entity.
#[test]
fn the_same_id_twice_is_one_match() {
    let found = select_lookup_match(
        LookupKind::State,
        "In Review",
        Some(TEAM),
        vec![
            state(SP_IN_REVIEW, "In Review", "SP"),
            state(SP_IN_REVIEW, "In Review", "SP"),
        ],
    )
    .expect("one entity is not ambiguous")
    .expect("the match resolves");
    assert_eq!(found.id, SP_IN_REVIEW);
}
