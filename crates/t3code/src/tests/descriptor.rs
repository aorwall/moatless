//! What the descriptor advertises, which is the contract a client reads before
//! it calls anything.

use super::fixtures::*;
use super::*;

/// The capability is how a client knows it may stop polling `sandbox.status`.
/// A server that serves the subscription but stays quiet about it leaves
/// every client on the poll.
#[tokio::test]
async fn the_descriptor_advertises_the_sandbox_status_push() {
    let frame = request("1", "server.getConfig", json!({}));
    let mut connection = connection();
    let output = connection.exchange(&frame, &state(), &viewer()).await;

    assert_eq!(
        output.frames[0]["exit"]["value"]["environment"]["capabilities"]["sandboxStatusPush"],
        json!(true)
    );
}

/// `agentStatus` is `optionalKey` and absent whenever the sandbox is not
/// up, so its absence from one answer says nothing about the server. The
/// capability is the only thing that distinguishes a server that reports it
/// from one that never will.
#[tokio::test]
async fn the_descriptor_advertises_the_sandbox_agent_status() {
    let frame = request("1", "server.getConfig", json!({}));
    let mut connection = connection();
    let output = connection.exchange(&frame, &state(), &viewer()).await;

    assert_eq!(
        output.frames[0]["exit"]["value"]["environment"]["capabilities"]["sandboxAgentStatus"],
        json!(true)
    );
}

/// `commands` is omitted when none are registered, which is most of the
/// time, so its absence from one answer says nothing about the server
/// either. Same reasoning as `sandboxAgentStatus` above.
#[tokio::test]
async fn the_descriptor_advertises_the_sandbox_commands() {
    let frame = request("1", "server.getConfig", json!({}));
    let mut connection = connection();
    let output = connection.exchange(&frame, &state(), &viewer()).await;

    assert_eq!(
        output.frames[0]["exit"]["value"]["environment"]["capabilities"]["sandboxCommands"],
        json!(true)
    );
}

/// The client gates both halves of following on this key: opening a thread by
/// link adds it to the listing, and the menu item takes it back out. Drop the
/// key and a viewer reaches neither.
#[tokio::test]
async fn the_descriptor_advertises_thread_follow() {
    let frame = request("1", "server.getConfig", json!({}));
    let mut connection = connection();
    let output = connection.exchange(&frame, &state(), &viewer()).await;

    assert_eq!(
        output.frames[0]["exit"]["value"]["environment"]["capabilities"]["threadFollow"],
        json!(true)
    );
}

/// The sidebar hides its filter entirely when this key is missing, and the
/// popover behind it is the only control that calls `threads.browse`. Drop the
/// key and a viewer has the method and no way to reach it.
#[tokio::test]
async fn the_descriptor_advertises_thread_browse() {
    let frame = request("1", "server.getConfig", json!({}));
    let mut connection = connection();
    let output = connection.exchange(&frame, &state(), &viewer()).await;

    assert_eq!(
        output.frames[0]["exit"]["value"]["environment"]["capabilities"]["threadBrowse"],
        json!(true)
    );
}

/// The two keys the composer reads before it decides how to send a file.
/// Reporting `attachmentUploads` moves it off the inline data-URL path for
/// images too, so a server that reports one without serving the other
/// loses the person their attachment.
#[tokio::test]
async fn the_descriptor_advertises_both_attachment_capabilities_together() {
    let frame = request("1", "server.getConfig", json!({}));
    let mut connection = connection();
    let output = connection.exchange(&frame, &state(), &viewer()).await;
    let capabilities = &output.frames[0]["exit"]["value"]["environment"]["capabilities"];

    assert_eq!(capabilities["attachmentUploads"], json!(true));
    assert_eq!(
        capabilities["fileAttachments"],
        json!({ "maxUploadBytes": 50 * 1024 * 1024 })
    );
}

/// Both keys are `optionalKey` in the contract, so a deployment that cannot
/// sign an upload URL has to omit them rather than answer `false` or
/// `null`: the client decodes the whole descriptor before it can connect.
#[test]
fn a_deployment_that_cannot_sign_an_upload_url_omits_both_keys() {
    let config = UiRpcConfig {
        environment_id: "moatless".into(),
        allowed_origins: vec![AllowedOrigin::parse("https://example.com")],
        otlp_traces_enabled: false,
        attachment_uploads: false,
        usage_limit_sources: false,
    };
    let capabilities = &descriptor(&config)["capabilities"];

    assert!(capabilities.get("attachmentUploads").is_none());
    assert!(capabilities.get("fileAttachments").is_none());
    assert!(capabilities.get("usageLimitSources").is_none());
}

/// The client drops `usageLimitSourcesUpdated` unless the descriptor declares
/// the capability.
#[test]
fn a_deployment_with_a_hub_declares_the_usage_limit_source_capability() {
    let config = UiRpcConfig {
        environment_id: "moatless".into(),
        allowed_origins: vec![AllowedOrigin::parse("https://example.com")],
        otlp_traces_enabled: false,
        attachment_uploads: false,
        usage_limit_sources: true,
    };

    assert_eq!(
        descriptor(&config)["capabilities"]["usageLimitSources"],
        true
    );
}

/// A client refuses a server whose version differs from its own, so the
/// descriptor answers with the contract constant.
#[test]
fn the_descriptor_reports_the_contract_version_the_code_implements() {
    let config = UiRpcConfig {
        environment_id: "moatless".into(),
        allowed_origins: vec![AllowedOrigin::parse("https://example.com")],
        otlp_traces_enabled: false,
        attachment_uploads: true,
        usage_limit_sources: false,
    };

    assert_eq!(
        descriptor(&config)["serverVersion"],
        json!(T3_CLIENT_CONTRACT_VERSION)
    );
    assert_ne!(
        T3_CLIENT_CONTRACT_VERSION,
        env!("CARGO_PKG_VERSION"),
        "the contract version tracks the T3 client, not this crate"
    );
}
