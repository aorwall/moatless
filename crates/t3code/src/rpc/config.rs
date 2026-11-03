use crate::attachments;
use crate::authentication::viewer_scope;
use crate::config::T3_CLIENT_CONTRACT_VERSION;
use crate::config::T3_ENVIRONMENT_LABEL;
use crate::config::UiRpcConfig;
use crate::oauth;
use crate::providers;
use crate::state::UiRpcState;
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use moatless_types::ui_rpc::UiRpcAuthMethod;
use serde_json::Value;
use serde_json::json;

/// Client surfaces this server cannot back, by the web client's `FeatureName`
/// in `apps/web/src/fork/features.ts`; the client hides each one listed. Drop a
/// name here when the methods behind it are served.
const UNSUPPORTED_SURFACES: &[&str] = &[
    "turnDiffs",
    "projectManagement",
    "workspaceSearchContents",
    "openInEditor",
    "serverAdministration",
    "providerConfiguration",
    "diagnostics",
    "connections",
    "assistantStreaming",
    "serverUpdateBanner",
    "projectGrouping",
    "accessMode",
    "worktreeSelection",
    "browserHistory",
    "snapShots",
    "checkpointFileRestore",
    "deviceHub",
    "scheduledTasks",
    "defaultModelSetting",
    "serverBehaviorSettings",
];

pub(crate) fn descriptor(config: &UiRpcConfig) -> Value {
    let mut descriptor = json!({
        "environmentId": config.environment_id,
        "label": T3_ENVIRONMENT_LABEL,
        "platform": {
            "os": platform_os(),
            "arch": platform_arch()
        },
        "serverVersion": T3_CLIENT_CONTRACT_VERSION,
        "capabilities": {
            "sideChats": true,
            "repositoryIdentity": false,
            "connectionProbe": true,
            "threadSettlement": true,
            "threadSnooze": true,
            // `thread.auto-settle.set`, and `autoSettleDisabledAt` on a row,
            // stored per viewer on their follow as the pin is.
            "threadAutoSettleOptOut": true,
            // `lastVisitedAt` is the viewer's read receipt, which
            // `thread.visit` and `thread.mark-unread` move.
            "threadVisitedTracking": true,
            // Whether `thread.visibility.set` is understood, and whether a
            // conversation row carries `visibility`. One flag for the pair: a
            // client that cannot read the current value has nothing to label
            // the control with.
            "threadVisibility": true,
            // Whether `thread.follow` and `thread.unfollow` are understood. A
            // listing here is the open work a viewer follows rather than every
            // thread there is, so a client that cannot follow has no way to put
            // a thread it reached by link into the sidebar, and no way out of
            // one it did.
            "threadFollow": true,
            // Whether `threads.browse` is understood. A listing is what this
            // viewer follows, so without this a client has no way to reach the
            // work of somebody who is not them, and must offer no filter for
            // it at all.
            "threadBrowse": true,
            // Whether a turn naming another harness's instance switches a
            // started thread to it. Absent, a client locks the provider picker
            // once the thread has started.
            "threadProviderSwitch": true,
            // Whether `sandbox.subscribeStatus` is understood. Absent, a client
            // has to poll `sandbox.status` to notice a sandbox that started
            // after its panel opened, so this is what lets it stop.
            "sandboxStatusPush": true,
            // Whether `sandbox.status` reports `agentStatus` alongside the
            // lifecycle state. Absent, a client cannot tell "this server does
            // not say" from "this agent is idle", so the panel has nothing to
            // show; present, the key is there whenever the sandbox is up.
            "sandboxAgentStatus": true,
            // Whether `sandbox.status` reports the commands registered through
            // `moat cmd`. The agent reports `idle` while one runs, so absent
            // this a client cannot tell a Task that is mid-build from one with
            // nothing left to do, and shows no command rows at all.
            "sandboxCommands": true,
            // Whether the sandbox panel's five methods are understood:
            // `sandbox.subscribeDetail`, `restart`, `redeploy`, `cleanup` and
            // `setIdleTimeout`. One flag for the set, because a client that has
            // any of them opens the panel and a client that has none of them
            // must not offer it at all.
            "sandboxDetail": true,
            // Whether `scripts.run` is understood. Scripts still list on the
            // project without this, but a client only shows the run control
            // where the server can run it. T3's own server omits this key
            // and answers `scripts.run` with `UnsupportedMethodError`.
            "workspaceScripts": true,
            // Whether `scripts.listForThread` is understood. A Task may declare
            // Scripts of its own, which its project does not carry and its
            // project's listing therefore cannot show; a client without this
            // key lists the project's alone and never asks.
            "taskScripts": true,
            // Whether `thread.pull-request.link` and `.unlink` are understood,
            // and whether a conversation row carries `pullRequests`. One flag
            // for the set: a client that cannot read the current links has
            // nothing to unlink, and `linkedPullRequest` beside them is the
            // single reference a client from before this key reads instead.
            "threadPullRequests": true,
            // `pullRequests.list` over each readable project's primary GitHub
            // repository, and `detail`, `reviewGuide`, `generateReviewGuide`
            // and the HTTP diff for its pull requests or a bound Task's.
            // Conversation and every write to the host are refused.
            "pullRequests": true,
            // `thread.pin`, `thread.unpin`, `thread.pin.reorder` and
            // `thread.active.reorder`, stored per viewer on their follow.
            "threadPinning": true,
            "threadPinReorder": true,
            "threadActiveReorder": true,
            // `regenerateTitle` on `thread.meta.update`, which asks the agent
            // to rename the Task. Absent, a client hides the action.
            "threadTitleRegeneration": true,
            // A turn's `context` records are understood: folded into its text
            // by `message_context`. Absent, a client folds them itself.
            "inlineMessageContext": true,
            // `thread.pull-request.watch` still mutes server-side; false
            // because no link carries `watch` for the toggle to read (see
            // docs/deferred-changes.md).
            "threadPullRequestWatch": false,
            // The `/api/v1` REST API and its cookie login share this origin.
            // Absent, the client offers T3 pairing and no administration pages.
            "moatlessApi": true,
            "unsupportedSurfaces": UNSUPPORTED_SURFACES
        }
    });

    // Both keys are `optionalKey`, absent rather than false when uploads
    // can't be served: `null` fails the client's descriptor decode outright.
    //
    // `attachmentUploads` moves the composer off the inline data-URL path
    // once a URL can be signed (see [`attachments`]); inline stays served
    // either way, for clients that haven't read it.
    if config.attachment_uploads {
        if let Some(capabilities) = descriptor["capabilities"].as_object_mut() {
            capabilities.insert("attachmentUploads".into(), json!(true));
            // What a turn may carry beyond images, and how large. Absent, a
            // client offers no file picker at all.
            capabilities.insert(
                "fileAttachments".into(),
                json!({ "maxUploadBytes": attachments::MAX_FILE_BYTES }),
            );
        }
    }

    // Absent rather than false for the same decode reason, and reported only
    // where a hub is configured: the client keeps a source event it has already
    // received until a snapshot without this key clears it, so a deployment
    // that lost its hub has to stop declaring the capability to clear the rows.
    if config.usage_limit_sources {
        if let Some(capabilities) = descriptor["capabilities"].as_object_mut() {
            capabilities.insert("usageLimitSources".into(), json!(true));
        }
    }

    // `optionalKey` in the contract, so a V1 deployment omits it rather than
    // sending `1`: a client that reads no key at all falls back to V1 the same
    // way one that predates negotiation entirely does.
    if config.serves_orchestration_v2() {
        descriptor["orchestrationProtocolVersion"] = json!(2);
        // Whether `message.dispatch` may omit the model and runtime context:
        // a V2 client that reads this sends the server-resolved dispatch modes.
        if let Some(capabilities) = descriptor["capabilities"].as_object_mut() {
            capabilities.insert("serverResolvedCommandContext".into(), json!(true));
        }
    }

    descriptor
}

pub(crate) fn auth_descriptor(state: &UiRpcState) -> Value {
    json!({
        "policy": "remote-reachable",
        // A pairing code is T3's `one-time-token`, and this is where a client
        // reads that pairing is on offer at all: the mobile app shows the flow
        // for what the environment says it accepts.
        "bootstrapMethods": ["one-time-token"],
        "sessionMethods": ["browser-session-cookie", "bearer-access-token"],
        "sessionCookieName": state.session_cookie_name.as_ref()
    })
}

/// What a client is told about the session it is already holding.
///
/// `sessionMethod` reports the credential this request actually arrived with,
/// not the one the browser would have used, so a paired device asking what it's
/// holding isn't told it's holding a cookie.
pub(crate) fn session_state(state: &UiRpcState, method: Option<UiRpcAuthMethod>) -> Value {
    json!({
        "authenticated": true,
        "auth": auth_descriptor(state),
        "scopes": oauth::GRANTED_SCOPES,
        "sessionMethod": match method {
            Some(UiRpcAuthMethod::SessionCookie) | None => "browser-session-cookie",
            Some(UiRpcAuthMethod::ApiKey) | Some(UiRpcAuthMethod::WebSocketTicket) => "bearer-access-token",
        }
    })
}

/// Now, in the format the contract's `IsoDateTime` decodes.
pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) async fn server_config(
    state: &UiRpcState,
    identity: &UiRpcAuthIdentity,
    workspace_snapshots: &[Value],
) -> Value {
    let agents = state.agents.list(&viewer_scope(identity)).await;
    // `otlpTracesEnabled` is this backend's own span export; `otlpTracesUrl` is
    // where the client's spans are forwarded ([`observability`]). Two knobs, and
    // the Settings panel's one sentence reads true only when both name the same
    // collector.
    let mut observability = json!({
        "logsDirectoryPath": "/moatless/logs",
        "localTracingEnabled": false,
        "otlpTracesEnabled": state.rpc.otlp_traces_enabled,
        "otlpMetricsEnabled": false
    });
    // The contract declares this key optional, not nullable, so an unset
    // collector omits it — a `null` would fail the client's config decode and
    // take the whole connection down with it.
    if let Some(endpoint) = state.client_traces.endpoint() {
        observability["otlpTracesUrl"] = json!(endpoint);
    }
    json!({
        "environment": descriptor(&state.rpc),
        "auth": auth_descriptor(state),
        "cwd": "/moatless",
        "keybindingsConfigPath": "/moatless/keybindings.json",
        "keybindings": [],
        "issues": [],
        "providers": providers::providers(
            &agents,
            &state.model_defaults,
            &now_rfc3339(),
            workspace_snapshots,
        ),
        "availableEditors": [],
        "observability": observability,
        // Partial: the client fills every other server setting with its
        // default. Guided reviews are written by a Task the viewer starts.
        // `providerInstances` enables Pi explicitly: T3 resolves a provider's
        // enabled state from settings, not from the provider's own `enabled`,
        // and treats an unconfigured Pi as off, which hides it from the picker.
        "settings": {
            "reviewGuidesEnabled": true,
            "providerInstances": { "pi": { "driver": "pi", "enabled": true } }
        },
        "shellResumeCompletionMarker": true,
        "threadResumeCompletionMarker": true
    })
}

/// The config stream's carrier for hub quota.
pub(crate) fn sources_event(sources: Value) -> Value {
    json!({
        "version": 1,
        "type": "usageLimitSourcesUpdated",
        "payload": { "sources": sources }
    })
}

pub(crate) fn platform_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}

pub(crate) fn platform_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "x64"
    } else {
        "other"
    }
}
