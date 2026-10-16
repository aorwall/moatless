//! The subscriptions this connection owns, and the one attach stream.
//!
//! A method here holds the frame loop; see `docs/deferred-changes.md`.

use super::sandbox::SandboxStatusInput;
use crate::authentication::viewer_scope;
use crate::config::T3_ENVIRONMENT_LABEL;
use crate::oauth::Scope;
use crate::rpc::config::descriptor;
use crate::rpc::config::server_config;
use crate::rpc::config::sources_event;
use crate::rpc::envelope::failure_exit;
use crate::rpc::errors::sandbox_failure_exit;
use crate::rpc::errors::shell_failure_exit;
use crate::rpc::errors::terminal_failure_exit;
use crate::rpc::session::ProtocolSession;
use crate::rpc::table::MethodTable;
use crate::rpc::table::Request;
use crate::rpc::watch;
use crate::sandbox;
use crate::snapshots::workspace_cwd;
use crate::subscriptions::OrchestrationV2SubscribeShellInput;
use crate::subscriptions::OrchestrationV2SubscribeThreadInput;
use crate::subscriptions::shell_subscription;
use crate::subscriptions::thread_subscription;
use crate::terminal;
use crate::terminal::TerminalInput;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Value;
use serde_json::json;
use tokio::sync::broadcast;
use tracing::error;

pub(super) fn stream_methods() -> MethodTable {
    MethodTable::new()
        // `subscribeDiscoveredLocalServers` is deliberately absent and answers
        // `UnsupportedMethodError`: there is no local-server discovery, and a
        // stream that never says anything reads to a client as "connected,
        // nothing to report".
        .stream(
            "orchestration.subscribeShell",
            Scope::OrchestrationRead,
            |session, request| Box::pin(session.orchestration_subscribe_shell(request)),
        )
        .stream(
            "orchestration.subscribeThread",
            Scope::OrchestrationRead,
            |session, request| Box::pin(session.orchestration_subscribe_thread(request)),
        )
        .stream(
            "sandbox.subscribeDetail",
            Scope::OrchestrationRead,
            |session, request| Box::pin(session.sandbox_subscribe_detail(request)),
        )
        .stream(
            "sandbox.subscribeStatus",
            Scope::OrchestrationRead,
            |session, request| Box::pin(session.sandbox_subscribe_status(request)),
        )
        .stream(
            "subscribeAuthAccess",
            Scope::AccessRead,
            |session, request| Box::pin(session.subscribe_auth_access(request)),
        )
        .stream(
            "subscribePreviewEvents",
            Scope::OrchestrationRead,
            |session, request| Box::pin(session.subscribe_preview_events(request)),
        )
        .stream(
            "subscribeServerConfig",
            Scope::OrchestrationRead,
            |session, request| Box::pin(session.subscribe_server_config(request)),
        )
        .stream(
            "subscribeServerLifecycle",
            Scope::OrchestrationRead,
            |session, request| Box::pin(session.subscribe_server_lifecycle(request)),
        )
        .stream(
            "subscribeTerminalEvents",
            Scope::TerminalOperate,
            |session, request| Box::pin(session.subscribe_terminal_events(request)),
        )
        .stream(
            "subscribeTerminalMetadata",
            Scope::TerminalOperate,
            |session, request| Box::pin(session.subscribe_terminal_metadata(request)),
        )
        .stream(
            "subscribeVcsStatus",
            Scope::OrchestrationRead,
            |session, request| Box::pin(session.subscribe_vcs_status(request)),
        )
        .stream(
            "terminal.attach",
            Scope::TerminalOperate,
            |session, request| Box::pin(session.terminal_attach(request)),
        )
}

/// The `subscribeServerConfig` input, of the opt-ins this backend reads.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ServerConfigSubscriptionInput {
    usage_limit_sources: Option<bool>,
}

/// The contract's `VcsStatusSubscriptionInput`, of the fields this backend
/// reads.
#[derive(Deserialize)]
pub(super) struct VcsStatusSubscriptionInput {
    cwd: Option<String>,
}

impl ProtocolSession {
    pub(super) async fn terminal_attach(&mut self, request: Request<TerminalInput>) -> Vec<Value> {
        let Request {
            id,
            payload,
            state,
            identity,
            ..
        } = request;
        let viewer = viewer_scope(&identity);
        let stream = match terminal::attach(
            state.reads.conversations(),
            state.sandboxes.as_ref(),
            &viewer,
            &payload,
        )
        .await
        {
            Ok(stream) => stream,
            Err(error) => return vec![terminal_failure_exit(&id, &payload, &error, &identity)],
        };
        let terminal = state.terminal.clone();
        self.streams.start_produced(&id, None, move |sink| {
            terminal::forward_attach_stream(stream, terminal, sink)
        })
    }

    pub(super) async fn sandbox_subscribe_status(
        &mut self,
        request: Request<SandboxStatusInput>,
    ) -> Vec<Value> {
        let Request {
            id,
            tag,
            scope,
            payload: SandboxStatusInput { thread_id: task_id },
            state,
            identity,
            ..
        } = request;
        let viewer = viewer_scope(&identity);
        // Seed from the same read the poller makes, so the indicator has a
        // status before the first interval elapses rather than after it.
        let seeded = match sandbox::status(&state.sandbox, &viewer, &task_id).await {
            Ok(status) => status,
            Err(error) => return vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
        };
        let sandbox = state.sandbox.clone();
        self.streams
            .start_produced(&id, Some(vec![seeded.clone()]), move |sink| {
                sandbox::watch_status(sink, scope, sandbox, viewer, task_id, seeded)
            })
    }

    pub(super) async fn sandbox_subscribe_detail(
        &mut self,
        request: Request<SandboxStatusInput>,
    ) -> Vec<Value> {
        let Request {
            id,
            tag,
            scope,
            payload: SandboxStatusInput { thread_id: task_id },
            state,
            identity,
            ..
        } = request;
        let viewer = viewer_scope(&identity);
        // Seeded for the same reason `sandbox.subscribeStatus` is, and the more
        // so: this subscription's interval is five times as long.
        let seeded = match sandbox::detail(&state.sandbox, &viewer, &task_id).await {
            Ok(detail) => detail,
            Err(error) => return vec![sandbox_failure_exit(&id, scope, tag, &error, &identity)],
        };
        let sandbox = state.sandbox.clone();
        self.streams
            .start_produced(&id, Some(vec![seeded.clone()]), move |sink| {
                sandbox::watch_detail(sink, scope, sandbox, viewer, task_id, seeded)
            })
    }

    pub(super) async fn orchestration_subscribe_shell(
        &mut self,
        request: Request<OrchestrationV2SubscribeShellInput>,
    ) -> Vec<Value> {
        let Request {
            id,
            payload,
            state,
            identity,
            shared,
            ..
        } = request;
        let viewer = viewer_scope(&identity);
        let subscription = shell_subscription(
            &state.reads,
            &state.model_defaults,
            &viewer,
            &state.environment,
            payload.after_sequence,
            shared.protocol,
        );
        let subscription = match state.reads.poll_gate().run(subscription).await {
            Ok(subscription) => subscription,
            Err(error) => return vec![shell_failure_exit(&id, &error, &identity)],
        };
        // Both halves come from `shell_subscription`: the items to send, and a
        // feed that already knows what sending them leaves the client holding.
        let mut values = subscription.values;
        if payload.request_completion_marker.unwrap_or(false) {
            // Last, after the snapshot or catch-up and before live events —
            // the contract's required order. The client reads it as
            // permission to stop rendering itself as synchronizing.
            values.push(json!({ "kind": "synchronized" }));
        }
        let reads = state.reads.clone();
        let model_defaults = state.model_defaults.clone();
        let listing = state.changes.listing();
        self.streams.start_produced(&id, Some(values), move |sink| {
            watch::watch_shell(
                sink,
                subscription.feed,
                reads,
                model_defaults,
                viewer,
                listing,
            )
        })
    }

    pub(super) async fn orchestration_subscribe_thread(
        &mut self,
        request: Request<OrchestrationV2SubscribeThreadInput>,
    ) -> Vec<Value> {
        let Request {
            id,
            payload,
            state,
            identity,
            ..
        } = request;
        let viewer = viewer_scope(&identity);
        let task_id = payload.thread_id;
        let deployment = state.deployment();
        let opened = thread_subscription(
            &state.reads,
            &deployment,
            &viewer,
            &task_id,
            payload.after_sequence,
            payload.request_completion_marker.unwrap_or(false),
        );
        let opened = match state.reads.poll_gate().run(opened).await {
            Ok(opened) => opened,
            Err(error) => {
                error!(
                    %error,
                    user_id = %identity.user_id,
                    task_id = %task_id,
                    "Failed to assemble T3 UI thread snapshot"
                );
                return vec![failure_exit(&id, "Failed to assemble the conversation")];
            }
        };
        let Some((values, feed)) = opened else {
            return vec![failure_exit(&id, "Conversation not found")];
        };
        let reads = state.reads.clone();
        let frontend_url = state.frontend_url.clone();
        let model_defaults = state.model_defaults.clone();
        let signals = state.changes.thread(&task_id);
        self.streams.start_produced(&id, Some(values), move |sink| {
            watch::watch_thread(
                sink,
                feed,
                reads,
                frontend_url,
                model_defaults,
                viewer,
                signals,
            )
        })
    }

    pub(super) async fn subscribe_server_config(
        &mut self,
        request: Request<ServerConfigSubscriptionInput>,
    ) -> Vec<Value> {
        let Request {
            id,
            payload,
            state,
            identity,
            shared,
            ..
        } = request;
        let snapshot = json!({
            "version": 1,
            "type": "snapshot",
            "config": server_config(
                &state,
                &identity,
                &shared.workspace_snapshots.published(),
            ).await
        });
        // A client that did not ask for `usageLimitSourcesUpdated` decodes the
        // stream against an event union without it and dies on one, so the
        // opt-in gates the whole producer rather than only the first frame.
        let sources_opted_in =
            state.rpc.usage_limit_sources && payload.usage_limit_sources.unwrap_or(false);
        let started = if sources_opted_in {
            let sources = state.usage_limit_sources.clone();
            let mut changes = sources.subscribe();
            let first = sources_event(sources.snapshot().await);
            self.streams
                .start_produced(&id, Some(vec![snapshot, first]), move |sink| async move {
                    loop {
                        match changes.recv().await {
                            Ok(next) => {
                                if !sink.send(sources_event(next)).await {
                                    return Ok(());
                                }
                            }
                            // The next tick republishes the whole set, so a
                            // dropped one costs a minute of staleness and
                            // nothing else.
                            Err(broadcast::error::RecvError::Lagged(_)) => {}
                            Err(broadcast::error::RecvError::Closed) => return Ok(()),
                        }
                    }
                })
        } else {
            self.streams.start(&id, Some(vec![snapshot]))
        };
        if self.streams.contains(id.as_str()) {
            shared.config_stream.open(&id);
        }
        started
    }

    pub(super) async fn subscribe_server_lifecycle(
        &mut self,
        request: Request<IgnoredAny>,
    ) -> Vec<Value> {
        self.streams.start(
            &request.id,
            Some(vec![json!({
                "version": 1,
                "sequence": 1,
                "type": "welcome",
                "payload": {
                    "environment": descriptor(&request.state.rpc),
                    "cwd": "/moatless",
                    "projectName": T3_ENVIRONMENT_LABEL
                }
            })]),
        )
    }

    pub(super) async fn subscribe_auth_access(
        &mut self,
        request: Request<IgnoredAny>,
    ) -> Vec<Value> {
        self.streams.start(
            &request.id,
            Some(vec![json!({
                "version": 1,
                "revision": 1,
                "type": "snapshot",
                "payload": {
                    "pairingLinks": [],
                    "clientSessions": []
                }
            })]),
        )
    }

    pub(super) async fn subscribe_preview_events(
        &mut self,
        request: Request<IgnoredAny>,
    ) -> Vec<Value> {
        let preview = request.state.preview.clone();
        let viewer = viewer_scope(&request.identity);
        self.streams.start_produced(&request.id, None, move |sink| {
            watch::watch_preview_events(sink, preview, viewer)
        })
    }

    pub(super) async fn subscribe_terminal_events(
        &mut self,
        request: Request<IgnoredAny>,
    ) -> Vec<Value> {
        let events = request.state.terminal.subscribe_events();
        let tasks = request.state.reads.tasks_arc();
        let viewer = viewer_scope(&request.identity);
        self.streams.start_produced(&request.id, None, move |sink| {
            watch::watch_terminal_events(sink, events, tasks, viewer)
        })
    }

    pub(super) async fn subscribe_terminal_metadata(
        &mut self,
        request: Request<IgnoredAny>,
    ) -> Vec<Value> {
        let hub = request.state.terminal.clone();
        let tasks = request.state.reads.tasks_arc();
        let viewer = viewer_scope(&request.identity);
        self.streams.start_produced(&request.id, None, move |sink| {
            watch::watch_terminal_metadata(sink, hub, tasks, viewer)
        })
    }

    pub(super) async fn subscribe_vcs_status(
        &mut self,
        request: Request<VcsStatusSubscriptionInput>,
    ) -> Vec<Value> {
        let Request {
            id,
            payload,
            state,
            identity,
            ..
        } = request;
        // A `cwd` this surface does not address is not a failure here: the
        // client opens this subscription for every thread it shows, including
        // ones with no sandbox, and a stream that says nothing is what "there is
        // no repository to report on" looks like.
        let Some(cwd) = workspace_cwd(payload.cwd.as_deref()) else {
            return self.streams.start(&id, None);
        };
        let conversations = state.reads.conversations_arc();
        let sandboxes = state.sandboxes.clone();
        let pull_requests = state.pull_requests.clone();
        let viewer = viewer_scope(&identity);
        self.streams.start_produced(&id, None, move |sink| {
            watch::watch_vcs_status(sink, cwd, conversations, sandboxes, pull_requests, viewer)
        })
    }
}
