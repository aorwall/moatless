//! What the descriptor advertises, which is the contract a client reads before
//! it calls anything.

use super::fixtures::*;
use super::*;

/// The capability is how a client knows it may stop polling `sandbox.status`. A
/// server that serves the subscription but stays quiet about it leaves every
/// client on the poll.
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

/// `agentStatus` is `optionalKey`, absent whenever the sandbox is down. One
/// absent answer says nothing; only the capability tells a server that reports
/// it apart from one that never will.
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

/// `commands` is omitted when none are registered, which is most of the time,
/// so its absence from one answer says nothing about the server either. Same
/// reasoning as `sandboxAgentStatus` above.
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

#[tokio::test]
async fn the_descriptor_advertises_thread_provider_switch() {
    let frame = request("1", "server.getConfig", json!({}));
    let mut connection = connection();
    let output = connection.exchange(&frame, &state(), &viewer()).await;

    assert_eq!(
        output.frames[0]["exit"]["value"]["environment"]["capabilities"]["threadProviderSwitch"],
        json!(true)
    );
}

/// A client without this key lists a thread's project's Scripts and never
/// sends `scripts.listForThread`, so a Script the agent declared on the Task
/// stays out of reach of the control that would run it.
#[tokio::test]
async fn the_descriptor_advertises_the_per_thread_script_listing() {
    let frame = request("1", "server.getConfig", json!({}));
    let mut connection = connection();
    let output = connection.exchange(&frame, &state(), &viewer()).await;

    assert_eq!(
        output.frames[0]["exit"]["value"]["environment"]["capabilities"]["taskScripts"],
        json!(true)
    );
}

/// One web build serves T3 servers too, and reads these two keys to tell this
/// one apart: without them it shows every surface and no Moatless login.
#[tokio::test]
async fn the_descriptor_declares_the_moatless_api_and_the_surfaces_it_cannot_back() {
    let frame = request("1", "server.getConfig", json!({}));
    let mut connection = connection();
    let output = connection.exchange(&frame, &state(), &viewer()).await;
    let capabilities = &output.frames[0]["exit"]["value"]["environment"]["capabilities"];

    assert_eq!(capabilities["moatlessApi"], json!(true));
    let surfaces = capabilities["unsupportedSurfaces"].as_array().unwrap();
    assert!(!surfaces.contains(&json!("mergeBack")));
    assert!(surfaces.contains(&json!("turnDiffs")));
    assert!(surfaces.contains(&json!("worktreeSelection")));
}

/// The two keys the composer reads before choosing how to send a file.
/// Reporting `attachmentUploads` also moves images off the inline data-URL
/// path; report both or the attachment is lost.
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
/// sign an upload URL has to omit them rather than answer `false` or `null`:
/// the client decodes the whole descriptor before it can connect.
#[test]
fn a_deployment_that_cannot_sign_an_upload_url_omits_both_keys() {
    let config = UiRpcConfig {
        environment_id: "moatless".into(),
        allowed_origins: vec![AllowedOrigin::parse("https://example.com")],
        otlp_traces_enabled: false,
        attachment_uploads: false,
        usage_limit_sources: false,
        scratch_workspace: None,
        linear_default_team: None,
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
        scratch_workspace: None,
        linear_default_team: None,
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
        scratch_workspace: None,
        linear_default_team: None,
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

/// The client refuses a server that reports no protocol version, and sends
/// server-resolved dispatch modes only where the descriptor declares them.
#[test]
fn the_descriptor_declares_the_protocol_and_server_resolved_context() {
    let config = UiRpcConfig {
        environment_id: "moatless".into(),
        allowed_origins: vec![AllowedOrigin::parse("https://example.com")],
        otlp_traces_enabled: false,
        attachment_uploads: false,
        usage_limit_sources: false,
        scratch_workspace: None,
        linear_default_team: None,
    };
    let descriptor = descriptor(&config);

    assert_eq!(descriptor["orchestrationProtocolVersion"], json!(2));
    assert_eq!(
        descriptor["capabilities"]["serverResolvedCommandContext"],
        json!(true)
    );
}
