//! What `get_config` publishes — providers, skills, the hub — and what an open
//! config stream sends after it.

use super::fixtures::*;
use super::*;

#[test]
fn gate_parser_accepts_enabled_values() {
    for value in ["1", "true", "TRUE", "yes", "on", " on "] {
        assert!(parse_enabled("T3_UI_RPC_ENABLED", value).unwrap());
    }
}

#[test]
fn gate_parser_accepts_disabled_values() {
    for value in ["0", "false", "FALSE", "no", "off", "", "  "] {
        assert!(!parse_enabled("T3_UI_RPC_ENABLED", value).unwrap());
    }
}

#[test]
fn gate_parser_rejects_invalid_values() {
    let error = parse_enabled("T3_UI_RPC_ENABLED", "sometimes").unwrap_err();

    assert!(matches!(
        error,
        UiRpcConfigError::Invalid(message)
            if message == "T3_UI_RPC_ENABLED must be true or false"
    ));
}

#[tokio::test]
async fn get_config_returns_minimal_decodable_wire_shape() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request("1", "server.getConfig", json!({})),
            &state(),
            &viewer(),
        )
        .await;
    assert_eq!(output.frames[0]["_tag"], "Exit");
    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(
        output.frames[0]["exit"]["value"]["environment"]["environmentId"],
        environment_id(&stub_identity()),
        "the reported environment id is the configured name and the \
         database's digest"
    );
    assert_eq!(
        output.frames[0]["exit"]["value"]["auth"]["sessionCookieName"],
        "moatless_session"
    );
    assert_eq!(
        output.frames[0]["exit"]["value"]["auth"]["policy"],
        "remote-reachable"
    );
    assert_eq!(
        output.frames[0]["exit"]["value"]["auth"]["bootstrapMethods"],
        json!(["one-time-token"]),
        "a client reads here that it may pair with a code"
    );
    assert_eq!(
        output.frames[0]["exit"]["value"]["auth"]["sessionMethods"],
        json!(["browser-session-cookie", "bearer-access-token"])
    );
    assert_eq!(
        output.frames[0]["exit"]["value"]["threadResumeCompletionMarker"], true,
        "do not advertise this unless subscribeThread can emit synchronized"
    );
}

/// The composer is disabled while the provider list is empty
/// (`ChatView.logic.ts` has nothing to resolve a model against), so this is the
/// frame that decides whether anybody can type at all.
#[tokio::test]
async fn get_config_publishes_one_provider_per_agent_kind() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request("1", "server.getConfig", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    let providers = output.frames[0]["exit"]["value"]["providers"]
        .as_array()
        .expect("the config carries a provider list");
    assert_eq!(
        providers
            .iter()
            .map(|provider| provider["instanceId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["claudeAgent", "codex"]
    );
    for provider in providers {
        assert!(
            !provider["models"].as_array().unwrap().is_empty(),
            "{} published no models, so its composer resolves nothing",
            provider["instanceId"]
        );
    }
}

/// T3 calls `refreshProviders` when a person opens the picker, and merges the
/// answer into the same state the config populated. Two lists that disagree
/// would move a thread's provider under it.
#[tokio::test]
async fn refreshing_providers_returns_what_the_config_already_published() {
    let mut connection = connection();
    let state = state();
    let config = connection
        .exchange(
            &request("1", "server.getConfig", json!({})),
            &state,
            &viewer(),
        )
        .await;
    let refreshed = connection
        .exchange(
            &request("2", "server.refreshProviders", json!({})),
            &state,
            &viewer(),
        )
        .await;

    let published = &config.frames[0]["exit"]["value"]["providers"];
    let refreshed = &refreshed.frames[0]["exit"]["value"]["providers"];
    // `checkedAt` is stamped per call and is the one field allowed to
    // differ; everything a client routes on has to match.
    let strip = |list: &Value| -> Vec<Value> {
        list.as_array()
            .unwrap()
            .iter()
            .map(|provider| {
                let mut provider = provider.clone();
                provider.as_object_mut().unwrap().remove("checkedAt");
                provider
            })
            .collect()
    };
    assert_eq!(strip(published), strip(refreshed));
}

/// The composer refreshes providers with the `cwd` it is about to send from,
/// and reads the skills for its `/` menu off the snapshot that comes back.
#[tokio::test]
async fn refreshing_providers_for_a_cwd_publishes_that_workspace_s_skills() {
    let mut connection = connection();
    let refreshed = connection
        .exchange(
            &request(
                "1",
                "server.refreshProviders",
                json!({ "cwd": "/moatless/workspaces/ws-1" }),
            ),
            &state(),
            &viewer(),
        )
        .await;

    let snapshot = &refreshed.frames[0]["exit"]["value"]["providers"][0]["workspaceSnapshots"][0];
    assert_eq!(snapshot["cwd"], "/moatless/workspaces/ws-1");
    assert_eq!(snapshot["skills"][0]["name"], "git-workflow");
    assert_eq!(snapshot["skills"][0]["description"], "Commit and PR flow");
    assert_eq!(snapshot["skills"][0]["scope"], "repo");
}

/// `usageLimitSources` is carried beside the config snapshot, so an opted-in
/// client sees the current hub quota as soon as it connects.
#[tokio::test]
async fn an_opted_in_config_subscriber_gets_the_hub_beside_the_snapshot() {
    let mut session = session();
    let mut state = state();
    state.rpc = Arc::new(UiRpcConfig {
        usage_limit_sources: true,
        ..UiRpcConfig::clone(&state.rpc)
    });
    state.usage_limit_sources = crate::usage_limit_sources::UsageLimitSources::fixed(json!([{
        "id": "cliproxy",
        "kind": "cliproxy",
        "label": "CLIProxyAPI",
        "checkedAt": "2026-09-06T12:00:00.000Z",
        "accounts": []
    }]));

    let subscribed = session
        .handle_json(
            &request(
                "1",
                "subscribeServerConfig",
                json!({ "usageLimitSources": true }),
            ),
            &state,
            &viewer(),
        )
        .await;

    let values = &subscribed.frames[0]["values"];
    assert_eq!(values[0]["type"], "snapshot");
    assert_eq!(values[1]["type"], "usageLimitSourcesUpdated");
    assert_eq!(values[1]["payload"]["sources"][0]["label"], "CLIProxyAPI");
}

/// A client that did not opt in decodes the stream against an event union
/// without this member, so one would take the whole connection down.
#[tokio::test]
async fn a_client_that_did_not_opt_in_is_sent_no_hub_event() {
    let mut session = session();
    let mut state = state();
    state.usage_limit_sources =
        crate::usage_limit_sources::UsageLimitSources::fixed(json!([{ "id": "cliproxy" }]));

    let subscribed = session
        .handle_json(
            &request("1", "subscribeServerConfig", json!({})),
            &state,
            &viewer(),
        )
        .await;

    assert_eq!(
        subscribed.frames[0]["values"].as_array().map(Vec::len),
        Some(1)
    );
}

/// The client's provider state is fed by `subscribeServerConfig` alone — the
/// `refreshProviders` reply only tells the composer whether to retry. A
/// snapshot that never reached the stream would leave the menu empty.
#[tokio::test]
async fn a_resolved_cwd_reaches_an_open_config_stream() {
    let mut connection = connection();
    let state = state();
    connection
        .exchange(
            &request("1", "subscribeServerConfig", json!({})),
            &state,
            &viewer(),
        )
        .await;
    let refreshed = connection
        .exchange(
            &request(
                "2",
                "server.refreshProviders",
                json!({ "cwd": "/moatless/workspaces/ws-1" }),
            ),
            &state,
            &viewer(),
        )
        .await;

    let pushed = &refreshed.frames[1];
    assert_eq!(pushed["_tag"], "Chunk");
    assert_eq!(pushed["requestId"], "1");
    assert_eq!(pushed["values"][0]["type"], "providerStatuses");
    assert_eq!(
        pushed["values"][0]["payload"]["providers"][0]["workspaceSnapshots"][0]["skills"][0]["name"],
        "git-workflow"
    );
}
