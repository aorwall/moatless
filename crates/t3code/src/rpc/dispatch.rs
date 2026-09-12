use crate::assets;
use crate::attachments;
use crate::authentication::viewer_scope;
use crate::commands;
use crate::config::T3_CLIENT_CONTRACT_VERSION;
use crate::config::T3_ENVIRONMENT_LABEL;
use crate::dao::ConversationBrowse;
use crate::files;
use crate::git_actions;
use crate::oauth;
use crate::preview;
use crate::projection;
use crate::providers;
use crate::review;
use crate::rpc::config::descriptor;
use crate::rpc::config::now_rfc3339;
use crate::rpc::config::platform_arch;
use crate::rpc::config::platform_os;
use crate::rpc::config::server_config;
use crate::rpc::config::sources_event;
use crate::rpc::frames::RequestId;
use crate::rpc::frames::authorization_error;
use crate::rpc::frames::branch_list_exit;
use crate::rpc::frames::branch_switch_result;
use crate::rpc::frames::connection_defect;
use crate::rpc::frames::failure_exit;
use crate::rpc::frames::file_failure_exit;
use crate::rpc::frames::git_action_failure_exit;
use crate::rpc::frames::preview_answer;
use crate::rpc::frames::pull_request_failure_exit;
use crate::rpc::frames::pull_request_ref;
use crate::rpc::frames::pull_request_summary_failure_exit;
use crate::rpc::frames::review_failure_exit;
use crate::rpc::frames::sandbox_failure_exit;
use crate::rpc::frames::scripts_failure_exit;
use crate::rpc::frames::servers_failure_exit;
use crate::rpc::frames::servers_not_found;
use crate::rpc::frames::shell_failure_exit;
use crate::rpc::frames::subtasks_failure_exit;
use crate::rpc::frames::success_exit;
use crate::rpc::frames::terminal_failure_exit;
use crate::rpc::frames::typed_failure_exit;
use crate::rpc::frames::unsupported_exit;
use crate::rpc::frames::valid_request_id;
use crate::rpc::metrics::answered;
use crate::rpc::session::ProtocolSession;
use crate::rpc::session::SEARCH_ENTRIES_LIMIT;
use crate::rpc::session::SEARCH_ENTRIES_MAX;
use crate::rpc::session::SEARCH_THREADS_LIMIT;
use crate::rpc::session::SEARCH_THREADS_MIN_QUERY;
use crate::rpc::session::SharedSession;
use crate::rpc::session::resolve_workspace_skills;
use crate::rpc::watch;
use crate::sandbox;
use crate::scripts;
use crate::servers;
use crate::servers::ServersError;
use crate::snapshots::after_sequence;
use crate::snapshots::archived_shell_snapshot;
use crate::snapshots::browse_snapshot;
use crate::snapshots::thread_shell;
use crate::snapshots::workspace_cwd;
use crate::state::UiRpcState;
use crate::subscriptions::shell_subscription;
use crate::subscriptions::thread_subscription;
use crate::subtasks;
use crate::subtasks::SubtasksError;
use crate::terminal;
use crate::usage;
use crate::workspace::WorkspaceAccessError;
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use serde_json::Value;
use serde_json::json;
use std::future::Future;
use std::future::ready;
use std::pin::Pin;
use tokio::sync::broadcast;
use tracing::error;
use tracing::warn;

/// One client request, owned by whatever answers it.
///
/// Owned rather than borrowed because a detached request outlives the frame-loop
/// call that routed it. Every field is an `Arc`, a short `String` or the `Value`
/// the frame already parsed, so building one costs a set of refcount bumps.
struct Request {
    id: RequestId,
    /// The method name, kept because the failure frames echo it back.
    tag: String,
    payload: Value,
    state: UiRpcState,
    identity: UiRpcAuthIdentity,
    shared: SharedSession,
}

impl ProtocolSession {
    /// Answer one request, or start it.
    ///
    /// `None`: the request needed nothing of this connection, so it was
    /// detached and answers on the frame channel once it finishes.
    pub(crate) async fn answer(
        &mut self,
        id: RequestId,
        tag: String,
        payload: Value,
        state: &UiRpcState,
        identity: &UiRpcAuthIdentity,
        span: &tracing::Span,
    ) -> Option<Vec<Value>> {
        if let Some(refusal) = self.refuse(&id, &tag, identity) {
            return Some(answered(span, &tag, identity, ready(refusal)).await);
        }
        let route = detach(Request {
            id: id.clone(),
            tag: tag.clone(),
            payload,
            state: state.clone(),
            identity: identity.clone(),
            shared: self.shared.clone(),
        });
        match route {
            Dispatch::Detached(request) => {
                let detached = {
                    let span = span.clone();
                    let tag = tag.clone();
                    let identity = identity.clone();
                    async move { answered(&span, &tag, &identity, request).await }
                };
                match self.detached.spawn(&id, detached) {
                    None => None,
                    // At the cap: answered on the frame loop, which is where
                    // every request was answered before any of them detached.
                    Some(request) => Some(request.await),
                }
            }
            Dispatch::Session(request) => {
                // Boxed like the detached path is: unboxed, every session
                // method's stack frame sits inside the frame loop's own future.
                let handled: Pin<Box<dyn Future<Output = Vec<Value>> + Send>> =
                    Box::pin(self.handle_request(*request));
                Some(answered(span, &tag, identity, handled).await)
            }
        }
    }

    /// The refusals that precede a dispatch, and so precede the choice of which
    /// path answers it.
    fn refuse(
        &mut self,
        id: &RequestId,
        tag: &str,
        identity: &UiRpcAuthIdentity,
    ) -> Option<Vec<Value>> {
        if !valid_request_id(id.as_str()) {
            return Some(vec![connection_defect(
                "invalid_request_id",
                "Request id is not a decimal string and cannot be correlated",
            )]);
        }
        if self.streams.contains_active(id) || self.detached.contains_active(id) {
            return Some(vec![connection_defect(
                "duplicate_request_id",
                format!("Duplicate in-flight RPC request id: {id}"),
            )]);
        }

        // The grant this deployment advertised, enforced. A client is told at
        // pairing which scopes it has; refusing here is what makes that
        // advertised list a description of the server rather than a wish.
        //
        // This is the deployment's grant, not the client's — see the plan's Out
        // of scope note. A pairing that asked for less than this still gets a
        // credential carrying all of it.
        let required = oauth::required_scope(tag);
        if !oauth::GRANTED_SCOPES.contains(&required) {
            // `warn!`, not `debug!`: deployments run at `RUST_LOG=info`, and a
            // client asking for a method this deployment withholds is the
            // refusal an operator most needs to see.
            warn!(%tag, %required, user_id = %identity.user_id, "T3 UI request refused; scope not granted");
            return Some(vec![typed_failure_exit(
                id,
                authorization_error(required, format!("This server does not grant {required}.")),
            )]);
        }
        None
    }

    /// Route the requests that need this connection: the streams it owns, and
    /// the transitions whose order is the client's order. Everything else is
    /// sorted out by [`detach`] before this is reached.
    async fn handle_request(&mut self, request: Request) -> Vec<Value> {
        // Cloned so an arm can take `request`: a `match` holds its scrutinee
        // borrowed across every arm, and each arm here hands the request on.
        let tag = request.tag.clone();
        match tag.as_str() {
            "server.probe" => server_probe(request),
            "server.getSettings" => server_get_settings(request),
            "server.reportClientActivity" => server_report_client_activity(request),
            "cloud.getRelayClientStatus" => cloud_get_relay_client_status(request),
            "orchestration.dispatchCommand" => orchestration_dispatch_command(request).await,
            "orchestration.replayEvents" => orchestration_replay_events(request),
            "vcs.switchRef" => vcs_switch_ref(request),
            "terminal.open" => terminal_open(request).await,
            "terminal.attach" => self.terminal_attach(request).await,
            "terminal.write" => terminal_write(request).await,
            "terminal.resize" => terminal_resize(request).await,
            "terminal.clear" => terminal_clear(request).await,
            "terminal.restart" => terminal_restart(request).await,
            "terminal.close" => terminal_close(request).await,
            "preview.open" => preview_open(request).await,
            "preview.navigate" => preview_navigate(request).await,
            "preview.resize" => preview_resize(request).await,
            "preview.refresh" => preview_refresh(request).await,
            "preview.reportStatus" => preview_report_status(request).await,
            "preview.close" => preview_close(request).await,
            "scripts.run" => scripts_run(request).await,
            "sandbox.start" => sandbox_start(request).await,
            "sandbox.stop" => sandbox_stop(request).await,
            "sandbox.restart" => sandbox_restart(request).await,
            "sandbox.redeploy" => sandbox_redeploy(request).await,
            "sandbox.cleanup" => sandbox_cleanup(request).await,
            "sandbox.setIdleTimeout" => sandbox_set_idle_timeout(request).await,
            "sandbox.subscribeStatus" => self.sandbox_subscribe_status(request).await,
            "sandbox.subscribeDetail" => self.sandbox_subscribe_detail(request).await,
            "subscribeServerStatus" => self.subscribe_server_status(request).await,
            "servers.subscribeLogs" => self.servers_subscribe_logs(request),
            "projects.writeFile" => projects_write_file(request).await,
            "assets.createUrl" => assets_create_url(request),
            "attachments.createUploadUrl" => attachments_create_upload_url(request),
            "attachments.delete" => attachments_delete(request).await,
            "orchestration.subscribeShell" => self.orchestration_subscribe_shell(request).await,
            "orchestration.subscribeThread" => self.orchestration_subscribe_thread(request).await,
            "subscribeServerConfig" => self.subscribe_server_config(request).await,
            "subscribeServerLifecycle" => self.subscribe_server_lifecycle(request),
            "subscribeAuthAccess" => self.subscribe_auth_access(request),
            "subscribePreviewEvents" => self.subscribe_preview_events(request),
            "subscribeTerminalEvents" => self.subscribe_terminal_events(request),
            "subscribeTerminalMetadata" => self.subscribe_terminal_metadata(request).await,
            "subscribeVcsStatus" => self.subscribe_vcs_status(request),
            "git.runStackedAction" => git_run_stacked_action(request).await,
            // `subscribeDiscoveredLocalServers` falls through here and says so:
            // there is no local-server discovery, and a stream that never says
            // anything reads to a client as "connected, nothing to report".
            _ => vec![unsupported_exit(&request.id, &request.tag)],
        }
    }

    async fn terminal_attach(&mut self, request: Request) -> Vec<Value> {
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

    async fn sandbox_subscribe_status(&mut self, request: Request) -> Vec<Value> {
        let Request {
            id,
            tag,
            payload,
            state,
            identity,
            ..
        } = request;
        let viewer = viewer_scope(&identity);
        let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
            return vec![failure_exit(
                &id,
                "sandbox.subscribeStatus requires a threadId",
            )];
        };
        let task_id = task_id.to_string();
        // Seed from the same read the poller makes, so the indicator has a
        // status before the first interval elapses rather than after it.
        let seeded = match sandbox::status(&state.sandbox, &viewer, &task_id).await {
            Ok(status) => status,
            Err(error) => return vec![sandbox_failure_exit(&id, &tag, &error, &identity)],
        };
        let sandbox = state.sandbox.clone();
        self.streams
            .start_produced(&id, Some(vec![seeded.clone()]), move |sink| {
                sandbox::watch_status(sink, sandbox, viewer, task_id, seeded)
            })
    }

    async fn sandbox_subscribe_detail(&mut self, request: Request) -> Vec<Value> {
        let Request {
            id,
            tag,
            payload,
            state,
            identity,
            ..
        } = request;
        let viewer = viewer_scope(&identity);
        let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
            return vec![failure_exit(
                &id,
                "sandbox.subscribeDetail requires a threadId",
            )];
        };
        let task_id = task_id.to_string();
        // Seeded for the same reason `sandbox.subscribeStatus` is, and the more
        // so: this subscription's interval is five times as long.
        let seeded = match sandbox::detail(&state.sandbox, &viewer, &task_id).await {
            Ok(detail) => detail,
            Err(error) => return vec![sandbox_failure_exit(&id, &tag, &error, &identity)],
        };
        let sandbox = state.sandbox.clone();
        self.streams
            .start_produced(&id, Some(vec![seeded.clone()]), move |sink| {
                sandbox::watch_detail(sink, sandbox, viewer, task_id, seeded)
            })
    }

    async fn subscribe_server_status(&mut self, request: Request) -> Vec<Value> {
        let Request {
            id,
            payload,
            state,
            identity,
            ..
        } = request;
        let viewer = viewer_scope(&identity);
        let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
            return vec![failure_exit(
                &id,
                "subscribeServerStatus requires a threadId",
            )];
        };
        let task_id = task_id.to_string();
        // Seed from the same read the poller makes, so the panel has a list
        // before the first interval elapses rather than after it.
        let seeded = match servers::list(&state.servers, &viewer, &task_id).await {
            Ok(listing) => json!({
                "threadId": task_id,
                "servers": listing.get("servers").cloned().unwrap_or(json!([])),
                "observedAt": chrono::Utc::now().to_rfc3339(),
            }),
            Err(ServersError::NotFound) => json!({
                "threadId": task_id,
                "servers": [],
                "observedAt": chrono::Utc::now().to_rfc3339(),
            }),
            Err(error) => return vec![servers_failure_exit(&id, &error, &identity)],
        };
        let servers = state.servers.clone();
        self.streams
            .start_produced(&id, Some(vec![seeded]), move |sink| {
                servers::watch_status(sink, servers, viewer, task_id)
            })
    }

    fn servers_subscribe_logs(&mut self, request: Request) -> Vec<Value> {
        let Request {
            id,
            payload,
            state,
            identity,
            ..
        } = request;
        let viewer = viewer_scope(&identity);
        let (Some(task_id), Some(name)) = (
            payload.get("threadId").and_then(Value::as_str),
            payload.get("name").and_then(Value::as_str),
        ) else {
            return vec![failure_exit(
                &id,
                "servers.subscribeLogs requires a threadId and a name",
            )];
        };
        let servers = state.servers.clone();
        let task_id = task_id.to_string();
        let name = name.to_string();
        let previous = payload
            .get("previous")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.streams.start_produced(&id, None, move |sink| {
            servers::watch_logs(sink, servers, viewer, task_id, name, previous)
        })
    }

    async fn orchestration_subscribe_shell(&mut self, request: Request) -> Vec<Value> {
        let Request {
            id,
            payload,
            state,
            identity,
            ..
        } = request;
        let viewer = viewer_scope(&identity);
        let subscription = match shell_subscription(
            &state.reads,
            &state.model_defaults,
            &viewer,
            &state.environment,
            after_sequence(&payload),
        )
        .await
        {
            Ok(subscription) => subscription,
            Err(error) => return vec![shell_failure_exit(&id, &error, &identity)],
        };
        // Both halves come from `shell_subscription`: the items to send, and a
        // feed that already knows what sending them leaves the client holding.
        let mut values = subscription.values;
        if payload
            .get("requestCompletionMarker")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            // Last, after the snapshot or the catch-up. The contract places the
            // marker "after the subscription has emitted its initial snapshot or
            // catch-up replay and before it begins emitting live events", and the
            // client reads it as permission to stop rendering itself as
            // synchronizing.
            values.push(json!({ "kind": "synchronized" }));
        }
        let reads = state.reads.clone();
        let model_defaults = state.model_defaults.clone();
        self.streams.start_produced(&id, Some(values), move |sink| {
            watch::watch_shell(sink, subscription.feed, reads, model_defaults, viewer)
        })
    }

    async fn orchestration_subscribe_thread(&mut self, request: Request) -> Vec<Value> {
        let Request {
            id,
            payload,
            state,
            identity,
            ..
        } = request;
        let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
            // The contract requires it and every client encodes against the
            // contract, so this is a malformed request rather than a
            // conversation that is missing.
            return vec![failure_exit(
                &id,
                "orchestration.subscribeThread requires a threadId",
            )];
        };
        let task_id = task_id.to_string();
        let viewer = viewer_scope(&identity);
        let opened = match thread_subscription(
            &state.reads,
            &state.deployment(),
            &viewer,
            &task_id,
            after_sequence(&payload),
            payload
                .get("requestCompletionMarker")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        )
        .await
        {
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
        self.streams.start_produced(&id, Some(values), move |sink| {
            watch::watch_thread(sink, feed, reads, frontend_url, viewer)
        })
    }

    async fn subscribe_server_config(&mut self, request: Request) -> Vec<Value> {
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
        let sources_opted_in = state.rpc.usage_limit_sources
            && payload
                .get("usageLimitSources")
                .and_then(Value::as_bool)
                .unwrap_or(false);
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

    fn subscribe_server_lifecycle(&mut self, request: Request) -> Vec<Value> {
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

    fn subscribe_auth_access(&mut self, request: Request) -> Vec<Value> {
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

    fn subscribe_preview_events(&mut self, request: Request) -> Vec<Value> {
        let preview = request.state.preview.clone();
        let viewer = viewer_scope(&request.identity);
        self.streams.start_produced(&request.id, None, move |sink| {
            watch::watch_preview_events(sink, preview, viewer)
        })
    }

    fn subscribe_terminal_events(&mut self, request: Request) -> Vec<Value> {
        let events = request.state.terminal.subscribe_events();
        self.streams.start_produced(&request.id, None, move |sink| {
            terminal::forward_value_subscription(events, sink)
        })
    }

    async fn subscribe_terminal_metadata(&mut self, request: Request) -> Vec<Value> {
        let snapshot = request.state.terminal.metadata_snapshot().await;
        let metadata = request.state.terminal.subscribe_metadata();
        self.streams
            .start_produced(&request.id, Some(vec![snapshot]), move |sink| {
                terminal::forward_value_subscription(metadata, sink)
            })
    }

    fn subscribe_vcs_status(&mut self, request: Request) -> Vec<Value> {
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
        let Some(cwd) = workspace_cwd(&payload) else {
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

/// Where one request is answered.
enum Dispatch {
    /// Off the frame loop, on a task of its own.
    Detached(Pin<Box<dyn Future<Output = Vec<Value>> + Send>>),
    /// On the frame loop, still holding everything it needs.
    Session(Box<Request>),
}

/// Route one request, building it where it detaches.
///
/// This match is the whole record of which methods answer off the frame loop: a
/// tag absent from it reaches [`ProtocolSession::handle_request`]. A method
/// belongs here when it only reads — anything that moves a named resource from
/// one state to another stays on the loop.
///
/// **What that buys, and what it costs.** The loop is sequential, so a
/// transition on it finishes before anything the client sent after it is even
/// dispatched — later in the same batch, or in any later frame. Read-your-writes
/// therefore survives detaching without a client-side convention. What does not
/// survive is the reverse: a detached read may now observe a transition the
/// client sent *after* it, and answers in no particular order against the
/// requests around it.
fn detach(request: Request) -> Dispatch {
    // Cloned so an arm can take `request`: a `match` holds its scrutinee
    // borrowed across every arm, and each arm here moves the request into the
    // future that answers it.
    let tag = request.tag.clone();
    let detached: Pin<Box<dyn Future<Output = Vec<Value>> + Send>> = match tag.as_str() {
        "server.getConfig" => Box::pin(server_get_config(request)),
        "server.refreshProviders" => Box::pin(server_refresh_providers(request)),
        "server.getUsageSummary" => Box::pin(server_get_usage_summary(request)),
        "server.discoverSourceControl" => Box::pin(server_discover_source_control(request)),
        "vcs.listRefs" => Box::pin(vcs_list_refs(request)),
        "vcs.refreshStatus" => Box::pin(vcs_refresh_status(request)),
        "review.getDiffPreview" => Box::pin(review_get_diff_preview(request)),
        "preview.list" => Box::pin(preview_list(request)),
        "servers.list" => Box::pin(servers_list(request)),
        "subtasks.list" => Box::pin(subtasks_list(request)),
        "sandbox.status" => Box::pin(sandbox_status(request)),
        "sandbox.listDiagnostics" => Box::pin(sandbox_list_diagnostics(request)),
        "projects.listEntries" => Box::pin(projects_list_entries(request)),
        "projects.readFile" => Box::pin(projects_read_file(request)),
        "projects.searchEntries" => Box::pin(projects_search_entries(request)),
        "orchestration.searchThreads" => Box::pin(orchestration_search_threads(request)),
        "threads.getShell" => Box::pin(threads_get_shell(request)),
        "threads.browse" => Box::pin(threads_browse(request)),
        "orchestration.getArchivedShellSnapshot" => {
            Box::pin(orchestration_get_archived_shell_snapshot(request))
        }
        "pullRequests.summary" => Box::pin(pull_requests_summary(request)),
        "git.resolvePullRequest" => Box::pin(git_resolve_pull_request(request)),
        "git.preparePullRequestThread" => Box::pin(git_prepare_pull_request_thread(request)),
        _ => return Dispatch::Session(Box::new(request)),
    };
    Dispatch::Detached(detached)
}

fn server_probe(request: Request) -> Vec<Value> {
    vec![success_exit(&request.id, json!({}))]
}

fn server_get_settings(request: Request) -> Vec<Value> {
    vec![success_exit(&request.id, json!({}))]
}

/// Which client is focused decides whether *T3* keeps background work running on
/// its own host. Moatless runs its agents in Sandboxes that no browser tab is
/// attached to, so the report is accepted and dropped rather than refused: the
/// client sends it on every focus change and a defect per change is pure noise.
fn server_report_client_activity(request: Request) -> Vec<Value> {
    vec![success_exit(&request.id, Value::Null)]
}

fn cloud_get_relay_client_status(request: Request) -> Vec<Value> {
    vec![success_exit(
        &request.id,
        json!({
            "status": "unsupported",
            "platform": platform_os(),
            "arch": platform_arch(),
            "version": T3_CLIENT_CONTRACT_VERSION
        }),
    )]
}

/// Still answers with nothing, and deliberately. This is not the path a
/// reconnecting client takes: no T3 client calls it — web, desktop and mobile
/// all share `client-runtime`, whose only resume is `subscribeShell` with
/// `afterSequence` (`state/shell.ts:181`), and `replayEvents` appears outside
/// `packages/contracts` only in the reference server and its own tests. Its
/// result type is `OrchestrationEvent`, which carries an event id, an aggregate,
/// a causation chain and a typed payload; a listing change record carries a
/// position and a Task id and nothing else, on purpose. Filling this in would
/// mean inventing an event log this backend does not have, for a method nothing
/// calls.
fn orchestration_replay_events(request: Request) -> Vec<Value> {
    vec![success_exit(&request.id, json!([]))]
}

async fn server_get_config(request: Request) -> Vec<Value> {
    let Request {
        id,
        state,
        identity,
        shared,
        ..
    } = request;
    let snapshots = shared.workspace_snapshots.published();
    vec![success_exit(
        &id,
        server_config(&state, &identity, &snapshots).await,
    )]
}

/// Publishes a fresh provider list twice: as the reply, which is what stops the
/// composer asking again, and on the open config stream, which is what fills the
/// menu.
async fn server_refresh_providers(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        shared,
        ..
    } = request;
    if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
        resolve_workspace_skills(&state, &identity, cwd, &shared.workspace_snapshots).await;
    }
    let agents = state.agents.list(&viewer_scope(&identity)).await;
    let providers = providers::providers(
        &agents,
        &state.model_defaults,
        &now_rfc3339(),
        &shared.workspace_snapshots.published(),
    );
    let mut frames = vec![success_exit(&id, json!({ "providers": providers.clone() }))];
    if let Some(stream) = shared.config_stream.opened() {
        frames.push(json!({
            "_tag": "Chunk",
            "requestId": stream.wire(),
            "values": [{
                "version": 1,
                "type": "providerStatuses",
                "payload": { "providers": providers }
            }]
        }));
    }
    frames
}

async fn server_get_usage_summary(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let window = match usage::window_from(&payload) {
        Ok(window) => window,
        Err(detail) => {
            return vec![typed_failure_exit(
                &id,
                json!({
                    "_tag": "UsageReadError",
                    "reason": "invalidWindow",
                    "detail": detail,
                }),
            )];
        }
    };
    let started = std::time::Instant::now();
    match state.usage.buckets(&viewer_scope(&identity), &window).await {
        Ok(rows) => {
            let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            vec![success_exit(
                &id,
                usage::summary(&window, &rows, &now_rfc3339(), elapsed),
            )]
        }
        // The cause is logged, not sent: it names tables and columns, and the
        // client renders `detail` verbatim.
        Err(cause) => {
            warn!(%tag, user_id = %identity.user_id, %cause, "T3 usage read failed");
            vec![typed_failure_exit(
                &id,
                json!({
                    "_tag": "UsageReadError",
                    "reason": "scanFailed",
                    "detail": "This deployment could not read usage.",
                }),
            )]
        }
    }
}

async fn server_discover_source_control(request: Request) -> Vec<Value> {
    let Request {
        id,
        state,
        identity,
        ..
    } = request;
    let github = state
        .source_control_auth
        .github_identity(&viewer_scope(&identity))
        .await;
    vec![success_exit(&id, git_actions::discovery(github.as_ref()))]
}

/// The one write. A refusal is a typed failure the composer renders, not a
/// defect: "that action does not exist here" is true, "this server is broken" is
/// not.
async fn orchestration_dispatch_command(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let ctx = commands::CommandContext {
        conversations: state.reads.conversations(),
        agents: state.agents.as_ref(),
        sink: state.commands.as_ref(),
        workspace_script_commands: state.workspace_script_commands.as_ref(),
        skills: state.skills.as_ref(),
        caller: identity.user_id,
    };
    match commands::dispatch(&ctx, &viewer_scope(&identity), &payload).await {
        Ok(result) => vec![success_exit(&id, result)],
        Err(refusal) => {
            // `warn!`, not `debug!`: this is the surface's only write, and at
            // `RUST_LOG=info` a refused one would otherwise leave nothing behind
            // at all.
            warn!(
                %tag,
                user_id = %identity.user_id,
                reason = %refusal.message(),
                "T3 UI command refused"
            );
            vec![typed_failure_exit(
                &id,
                json!({
                    "_tag": "OrchestrationDispatchCommandError",
                    "message": refusal.message(),
                }),
            )]
        }
    }
}

async fn orchestration_search_threads(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let query = payload
        .get("query")
        .and_then(Value::as_str)
        .map_or("", str::trim);
    // See [`SEARCH_THREADS_MIN_QUERY`]. Answering an empty list is the honest
    // response to "match anything", and it costs nothing.
    if query.chars().count() < SEARCH_THREADS_MIN_QUERY {
        return vec![success_exit(&id, json!({ "matches": [] }))];
    }
    let limit = payload
        .get("limit")
        .and_then(Value::as_i64)
        .filter(|limit| *limit > 0)
        .map_or(SEARCH_THREADS_LIMIT, |limit| {
            limit.min(SEARCH_THREADS_LIMIT)
        });
    match state
        .reads
        .conversations()
        .search_conversations(&viewer_scope(&identity), query, limit)
        .await
    {
        Ok(matches) => vec![success_exit(
            &id,
            json!({ "matches": projection::thread_matches(&matches, query) }),
        )],
        Err(error) => {
            error!(%error, user_id = %identity.user_id, "T3 UI thread search failed");
            vec![typed_failure_exit(
                &id,
                json!({
                    "_tag": "OrchestrationSearchThreadsError",
                    "message": "Failed to search conversations.",
                }),
            )]
        }
    }
}

/// The listings' complement: one row, by id, for a conversation the client is
/// holding and no listing named. `null` is the answer for every way it can be
/// absent — the client is already on that page, and a failure would take a
/// rendered transcript off the screen to report a missing title.
async fn threads_get_shell(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
        return vec![failure_exit(&id, "threads.getShell requires a threadId")];
    };
    match thread_shell(&state.reads, &state.model_defaults, &viewer, task_id).await {
        Ok(thread) => vec![success_exit(
            &id,
            json!({ "thread": thread.unwrap_or(Value::Null) }),
        )],
        Err(error) => vec![shell_failure_exit(&id, &error, &identity)],
    }
}

/// The listing rows a filter names, for conversations outside the viewer's own
/// listing.
///
/// The refusal is the filter being empty, not the viewer not being an
/// administrator: what a browse may reach is the read rule, which
/// `list_browsable` applies to everyone alike. Empty is refused because that
/// read unnarrowed is every Task an administrator may read.
async fn threads_browse(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let text = |key: &str| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let Some(filter) = ConversationBrowse::new(
        text("ownerUserId"),
        text("tag"),
        payload
            .get("includeClosed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    ) else {
        warn!(
            user_id = %viewer.user_id,
            "T3 UI thread browse refused; it names neither an owner nor a tag"
        );
        return vec![failure_exit(
            &id,
            "threads.browse requires an ownerUserId or a tag",
        )];
    };
    match browse_snapshot(&state.reads, &state.model_defaults, &viewer, &filter).await {
        Ok(threads) => vec![success_exit(&id, threads)],
        Err(error) => vec![shell_failure_exit(&id, &error, &identity)],
    }
}

async fn orchestration_get_archived_shell_snapshot(request: Request) -> Vec<Value> {
    let Request {
        id,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match archived_shell_snapshot(&state.reads, &state.model_defaults, &viewer).await {
        Ok(snapshot) => vec![success_exit(&id, snapshot)],
        Err(error) => vec![shell_failure_exit(&id, &error, &identity)],
    }
}

/// A draft thread has no checkout to switch: its branch is recorded on the draft
/// and applied when the Task is created, so the switch is answered as done,
/// echoing the ref the client should store. A thread that already has a Sandbox
/// cannot change branch — that is fixed at creation — so the switch there falls
/// through to unsupported.
fn vcs_switch_ref(request: Request) -> Vec<Value> {
    match branch_switch_result(&request.payload) {
        Some(result) => vec![success_exit(&request.id, result)],
        None => vec![unsupported_exit(&request.id, &request.tag)],
    }
}

async fn vcs_list_refs(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some(cwd) = workspace_cwd(&payload) else {
        return vec![success_exit(&id, review::no_refs())];
    };
    let refs = review::RefRequest::from_payload(&cwd, &payload);
    // A draft thread has no Sandbox yet and addresses its Workspace by the
    // project root. Its branches are the primary repository's, read from the git
    // host — the same list the Moatless branch picker shows before a Task runs.
    if projection::is_project_root(&cwd) {
        return match review::workspace_branches(
            state.reads.workspaces(),
            state.branches.as_ref(),
            &viewer_scope(&identity),
            &cwd,
            &refs,
        )
        .await
        {
            Ok(refs) => vec![success_exit(&id, refs)],
            Err(error) => vec![branch_list_exit(&id, &cwd, error, &identity)],
        };
    }
    match review::list_refs(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &viewer_scope(&identity),
        &refs,
    )
    .await
    {
        Ok(refs) => vec![success_exit(&id, refs)],
        Err(error) if review::refs_are_quiet(&error) => vec![success_exit(&id, review::no_refs())],
        Err(error) => vec![review_failure_exit(&id, &tag, &cwd, &error, &identity)],
    }
}

async fn vcs_refresh_status(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some(cwd) = workspace_cwd(&payload) else {
        // No cwd is why this failed, so there is none to report.
        return vec![review_failure_exit(
            &id,
            &tag,
            "",
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    match review::status_result(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        state.pull_requests.as_ref(),
        &viewer_scope(&identity),
        &cwd,
    )
    .await
    {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![review_failure_exit(&id, &tag, &cwd, &error, &identity)],
    }
}

async fn review_get_diff_preview(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some(cwd) = workspace_cwd(&payload) else {
        // No cwd is why this failed, so there is none to report.
        return vec![review_failure_exit(
            &id,
            &tag,
            "",
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    match review::diff_preview(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &viewer_scope(&identity),
        &cwd,
        payload
            .get("baseRef")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|base| !base.is_empty()),
        payload
            .get("ignoreWhitespace")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
    .await
    {
        Ok(preview) => vec![success_exit(&id, preview)],
        Err(error) => vec![review_failure_exit(&id, &tag, &cwd, &error, &identity)],
    }
}

/// The one method of the `pullRequests.*` group this backend serves. The rest of
/// the group reads a git host and is refused, which is why
/// `capabilities.pullRequests` stays absent: the listing route, the right-panel
/// surface and the review actions all gate on it, and none of them can be served
/// from a binding.
async fn pull_requests_summary(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some((project_id, repository, number)) = pull_request_ref(&payload) else {
        return vec![pull_request_summary_failure_exit(
            &id,
            "A pull request reference needs a projectId, repository and number.",
        )];
    };
    match review::pull_request_summary(
        state.reads.conversations(),
        state.pull_requests.as_ref(),
        &viewer_scope(&identity),
        &project_id,
        &repository,
        number,
    )
    .await
    {
        Ok(summary) => vec![success_exit(&id, summary)],
        Err(review::PullRequestSummaryError::Unknown) => {
            vec![pull_request_summary_failure_exit(
                &id,
                "No pull request is linked to a task you can read.",
            )]
        }
        Err(review::PullRequestSummaryError::Store(error)) => {
            error!(%error, user_id = %identity.user_id, "T3 pull-request summary read failed");
            vec![pull_request_summary_failure_exit(
                &id,
                "The pull request could not be read.",
            )]
        }
    }
}

async fn git_resolve_pull_request(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    match git_actions::resolve_pull_request(
        state.reads.workspaces(),
        state.reads.conversations(),
        state.pull_requests.as_ref(),
        &viewer_scope(&identity),
        &payload,
    )
    .await
    {
        Ok(result) => vec![success_exit(&id, result)],
        Err(error) => vec![pull_request_failure_exit(&id, &tag, &error, &identity)],
    }
}

async fn git_prepare_pull_request_thread(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    match git_actions::prepare_pull_request_thread(
        state.reads.workspaces(),
        state.reads.conversations(),
        state.pull_requests.as_ref(),
        &viewer_scope(&identity),
        &payload,
    )
    .await
    {
        Ok(git_actions::PreparedThread::Local(result)) => vec![success_exit(&id, result)],
        // Worktree mode is refused by name, not by falling through to the
        // catch-all: this surface prepares a branch, never a worktree on disk.
        Ok(git_actions::PreparedThread::WorktreeUnsupported) => vec![unsupported_exit(&id, &tag)],
        Err(error) => vec![pull_request_failure_exit(&id, &tag, &error, &identity)],
    }
}

/// The action is not run here or in the Sandbox: the button press becomes a
/// message asking the Task's agent to do the git work. The client's contract is
/// a stream, so it is answered with one `action_finished` chunk and a terminal
/// success, the same shape a completed action produced.
async fn git_run_stacked_action(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    match git_actions::run_stacked_action(
        state.reads.conversations(),
        state.commands.as_ref(),
        &viewer_scope(&identity),
        &payload,
    )
    .await
    {
        Ok(sent) => vec![
            json!({
                "_tag": "Chunk",
                "requestId": id.wire(),
                "values": [git_actions::asked_agent_event(&sent)],
            }),
            success_exit(&id, Value::Null),
        ],
        Err(error) => vec![git_action_failure_exit(&id, &error, &identity)],
    }
}

async fn terminal_open(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match terminal::open(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &state.terminal,
        &viewer,
        &payload,
    )
    .await
    {
        Ok(snapshot) => vec![success_exit(&id, snapshot)],
        Err(error) => vec![terminal_failure_exit(&id, &payload, &error, &identity)],
    }
}

async fn terminal_write(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match terminal::write(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &viewer,
        &payload,
    )
    .await
    {
        Ok(()) => vec![success_exit(&id, Value::Null)],
        Err(error) => vec![terminal_failure_exit(&id, &payload, &error, &identity)],
    }
}

async fn terminal_resize(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match terminal::resize(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &viewer,
        &payload,
    )
    .await
    {
        Ok(()) => vec![success_exit(&id, Value::Null)],
        Err(error) => vec![terminal_failure_exit(&id, &payload, &error, &identity)],
    }
}

async fn terminal_clear(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match terminal::clear(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &state.terminal,
        &viewer,
        &payload,
    )
    .await
    {
        Ok(()) => vec![success_exit(&id, Value::Null)],
        Err(error) => vec![terminal_failure_exit(&id, &payload, &error, &identity)],
    }
}

async fn terminal_restart(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match terminal::restart(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &state.terminal,
        &viewer,
        &payload,
    )
    .await
    {
        Ok(snapshot) => vec![success_exit(&id, snapshot)],
        Err(error) => vec![terminal_failure_exit(&id, &payload, &error, &identity)],
    }
}

async fn terminal_close(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match terminal::close(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &state.terminal,
        &viewer,
        &payload,
    )
    .await
    {
        Ok(()) => vec![success_exit(&id, Value::Null)],
        Err(error) => vec![terminal_failure_exit(&id, &payload, &error, &identity)],
    }
}

async fn preview_list(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        preview::list(&state.preview, &viewer_scope(&identity), &payload).await,
    )
}

async fn preview_open(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        preview::open(&state.preview, &viewer_scope(&identity), &payload).await,
    )
}

async fn preview_navigate(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        preview::navigate(&state.preview, &viewer_scope(&identity), &payload).await,
    )
}

async fn preview_resize(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        preview::resize(&state.preview, &viewer_scope(&identity), &payload).await,
    )
}

/// The three void methods answer `null` rather than a snapshot, matching the
/// contract's declared success type.
async fn preview_refresh(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        preview::refresh(&state.preview, &viewer_scope(&identity), &payload)
            .await
            .map(|()| Value::Null),
    )
}

async fn preview_report_status(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        preview::report_status(&state.preview, &viewer_scope(&identity), &payload)
            .await
            .map(|()| Value::Null),
    )
}

async fn preview_close(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        preview::close(&state.preview, &viewer_scope(&identity), &payload)
            .await
            .map(|()| Value::Null),
    )
}

async fn scripts_run(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match scripts::run(
        &state.scripts,
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &state.terminal,
        &viewer,
        &payload,
    )
    .await
    {
        Ok(result) => vec![success_exit(&id, result)],
        Err(error) => vec![scripts_failure_exit(&id, &error, &payload, &identity)],
    }
}

async fn sandbox_start(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
        return vec![failure_exit(&id, "sandbox.start requires a threadId")];
    };
    match sandbox::start(&state.sandbox, &viewer, task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, &tag, &error, &identity)],
    }
}

async fn sandbox_stop(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
        return vec![failure_exit(&id, "sandbox.stop requires a threadId")];
    };
    match sandbox::stop(&state.sandbox, &viewer, task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, &tag, &error, &identity)],
    }
}

async fn sandbox_restart(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
        return vec![failure_exit(&id, "sandbox.restart requires a threadId")];
    };
    match sandbox::restart(&state.sandbox, &viewer, task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, &tag, &error, &identity)],
    }
}

async fn sandbox_redeploy(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
        return vec![failure_exit(&id, "sandbox.redeploy requires a threadId")];
    };
    match sandbox::redeploy(&state.sandbox, &viewer, task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, &tag, &error, &identity)],
    }
}

async fn sandbox_cleanup(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
        return vec![failure_exit(&id, "sandbox.cleanup requires a threadId")];
    };
    match sandbox::cleanup(&state.sandbox, &viewer, task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, &tag, &error, &identity)],
    }
}

async fn sandbox_set_idle_timeout(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
        return vec![failure_exit(
            &id,
            "sandbox.setIdleTimeout requires a threadId",
        )];
    };
    // `as_i64` and not `as_u64`: a client that sends a negative gets the
    // refusal `sandbox::set_idle_timeout` states, not "requires minutes".
    let Some(minutes) = payload
        .get("minutes")
        .and_then(Value::as_i64)
        .and_then(|minutes| i32::try_from(minutes).ok())
    else {
        return vec![failure_exit(
            &id,
            "sandbox.setIdleTimeout requires minutes",
        )];
    };
    match sandbox::set_idle_timeout(state.commands.as_ref(), &viewer, task_id, minutes).await {
        Ok(result) => vec![success_exit(&id, result)],
        Err(error) => vec![sandbox_failure_exit(&id, &tag, &error, &identity)],
    }
}

async fn sandbox_status(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
        return vec![failure_exit(&id, "sandbox.status requires a threadId")];
    };
    match sandbox::status(&state.sandbox, &viewer, task_id).await {
        Ok(status) => vec![success_exit(&id, status)],
        Err(error) => vec![sandbox_failure_exit(&id, &tag, &error, &identity)],
    }
}

async fn sandbox_list_diagnostics(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        state,
        identity,
        ..
    } = request;
    match sandbox::list_diagnostics(&state.sandbox).await {
        Ok(entries) => vec![success_exit(&id, entries)],
        Err(error) => vec![sandbox_failure_exit(&id, &tag, &error, &identity)],
    }
}

async fn servers_list(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
        return vec![failure_exit(&id, "servers.list requires a threadId")];
    };
    match servers::list(&state.servers, &viewer, task_id).await {
        Ok(listing) => vec![success_exit(&id, listing)],
        // See [`servers_not_found`]: an unprovisioned conversation has no
        // servers, which is an answer and not a failure.
        Err(ServersError::NotFound) => vec![success_exit(&id, servers_not_found())],
        Err(error) => vec![servers_failure_exit(&id, &error, &identity)],
    }
}

async fn subtasks_list(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
        return vec![failure_exit(&id, "subtasks.list requires a threadId")];
    };
    match subtasks::list(&state.subtasks, &viewer, task_id).await {
        Ok(listing) => vec![success_exit(&id, listing)],
        // A Thread the viewer cannot read has no subtasks *for them*, and the
        // panel is a section on a page they are already looking at: an empty
        // list is the honest answer and a failure would red-flag a working page.
        Err(SubtasksError::NotFound) => vec![success_exit(&id, json!({ "subtasks": [] }))],
        Err(error) => vec![subtasks_failure_exit(&id, &error, &identity)],
    }
}

async fn projects_list_entries(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let Some(cwd) = workspace_cwd(&payload) else {
        return vec![file_failure_exit(
            &id,
            &tag,
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    match files::list_entries(
        &state.reads,
        state.file_sources(),
        &viewer_scope(&identity),
        &cwd,
    )
    .await
    {
        Ok(listing) => vec![success_exit(&id, listing)],
        Err(error) => vec![file_failure_exit(&id, &tag, &error, &identity)],
    }
}

async fn projects_read_file(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let relative_path = payload
        .get("relativePath")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty());
    let (Some(cwd), Some(relative_path)) = (workspace_cwd(&payload), relative_path) else {
        return vec![file_failure_exit(
            &id,
            &tag,
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    match files::read_file(
        &state.reads,
        state.file_sources(),
        &viewer_scope(&identity),
        &cwd,
        relative_path,
    )
    .await
    {
        Ok(file) => vec![success_exit(&id, file)],
        Err(error) => vec![file_failure_exit(&id, &tag, &error, &identity)],
    }
}

async fn projects_search_entries(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    // An absent or blank query is legal and means "show me what is here": the
    // picker opens before anyone types. `search_entries` answers that with a
    // bounded listing rather than nothing, because the Sandbox's name search
    // matches no path at all for an empty needle and an empty picker looks like
    // a broken one.
    let query = payload
        .get("query")
        .and_then(Value::as_str)
        .map_or("", str::trim);
    let Some(cwd) = workspace_cwd(&payload) else {
        return vec![file_failure_exit(
            &id,
            &tag,
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    let limit = payload
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(SEARCH_ENTRIES_LIMIT, |limit| {
            (limit as usize).min(SEARCH_ENTRIES_MAX)
        });
    match files::search_entries(
        &state.reads,
        state.file_sources(),
        &viewer_scope(&identity),
        &cwd,
        query,
        limit,
        payload.get("kind").and_then(Value::as_str),
    )
    .await
    {
        Ok(entries) => vec![success_exit(&id, entries)],
        Err(error) => vec![file_failure_exit(&id, &tag, &error, &identity)],
    }
}

async fn projects_write_file(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let relative_path = payload
        .get("relativePath")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty());
    // Empty contents are a legal write — truncating a file is a thing a person
    // does — so only an absent `contents` is unaddressable.
    let contents = payload.get("contents").and_then(Value::as_str);
    let (Some(cwd), Some(relative_path), Some(contents)) =
        (workspace_cwd(&payload), relative_path, contents)
    else {
        return vec![file_failure_exit(
            &id,
            &tag,
            &WorkspaceAccessError::Unaddressable,
            &identity,
        )];
    };
    match files::write_file(
        &state.reads,
        state.sandboxes.as_ref(),
        &viewer_scope(&identity),
        &cwd,
        relative_path,
        contents,
    )
    .await
    {
        Ok(written) => vec![success_exit(&id, written)],
        Err(error) => vec![file_failure_exit(&id, &tag, &error, &identity)],
    }
}

/// A conversation's checkout exists inside its Sandbox and nowhere else, so an
/// asset is minted as a route this backend serves the file on rather than as a
/// path on this machine — see [`assets`]. `workspace-file` and `media-file` both
/// name a file in that checkout. An attachment is an upload and is minted at the
/// upload route. A project's favicon names no file here and keeps answering that
/// there is none, which is what the client renders as a project without an icon.
fn assets_create_url(request: Request) -> Vec<Value> {
    let Request { id, payload, .. } = request;
    // The error carries the resource back, and the client decodes it against the
    // same schema it sent — so a request without one has no typed answer to give
    // and is a malformed request instead.
    let Some(resource) = payload.get("resource") else {
        return vec![failure_exit(&id, "assets.createUrl requires a resource")];
    };
    match assets::create_url(resource) {
        Ok(minted) => vec![success_exit(&id, minted)],
        Err(error) => vec![typed_failure_exit(
            &id,
            json!({
                "_tag": error.tag(),
                "resource": resource
            }),
        )],
    }
}

/// Where the composer sends a file before it sends the turn naming it. The URL
/// carries its own credential and the route serves it — see [`attachments`] — so
/// nothing about the upload reaches this socket except the id it will be known
/// by.
fn attachments_create_upload_url(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    match attachments::create_upload_url(&identity, state.session_jwt_secret.as_deref(), &payload) {
        Ok(minted) => vec![success_exit(&id, minted)],
        Err(attachments::MintError::Invalid(message)) => vec![failure_exit(&id, message)],
        Err(attachments::MintError::NoSigningKey) => vec![typed_failure_exit(
            &id,
            json!({
                "_tag": "AttachmentUploadSigningKeyError",
                "cause": "This deployment cannot sign attachment upload URLs."
            }),
        )],
    }
}

async fn attachments_delete(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    match attachments::delete(
        state.attachments.as_ref(),
        &viewer_scope(&identity),
        &payload,
    )
    .await
    {
        Ok(()) => vec![success_exit(&id, Value::Null)],
        Err(message) => vec![failure_exit(&id, message)],
    }
}
