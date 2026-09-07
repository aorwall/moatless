use crate::assets;
use crate::attachments;
use crate::authentication::viewer_scope;
use crate::commands;
use crate::config::T3_CLIENT_CONTRACT_VERSION;
use crate::config::T3_ENVIRONMENT_LABEL;
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
use crate::rpc::session::ProtocolSession;
use crate::rpc::session::SEARCH_ENTRIES_LIMIT;
use crate::rpc::session::SEARCH_ENTRIES_MAX;
use crate::rpc::session::SEARCH_THREADS_LIMIT;
use crate::rpc::session::SEARCH_THREADS_MIN_QUERY;
use crate::rpc::watch;
use crate::sandbox;
use crate::scripts;
use crate::servers;
use crate::servers::ServersError;
use crate::snapshots::after_sequence;
use crate::snapshots::archived_shell_snapshot;
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
use tokio::sync::broadcast;
use tracing::error;
use tracing::warn;

impl ProtocolSession {
    pub(crate) async fn handle_request(
        &mut self,
        id: RequestId,
        tag: String,
        payload: Value,
        state: &UiRpcState,
        identity: &UiRpcAuthIdentity,
    ) -> Vec<Value> {
        if !valid_request_id(id.as_str()) {
            return vec![connection_defect(
                "invalid_request_id",
                "Request id is not a decimal string and cannot be correlated",
            )];
        }
        if self.streams.contains_active(&id) {
            return vec![connection_defect(
                "duplicate_request_id",
                format!("Duplicate in-flight RPC request id: {id}"),
            )];
        }

        // The grant this deployment advertised, enforced. A client is told at
        // pairing which scopes it has; refusing here is what makes that
        // advertised list a description of the server rather than a wish.
        //
        // This is the deployment's grant, not the client's — see the plan's Out
        // of scope note. A pairing that asked for less than this still gets a
        // credential carrying all of it.
        let required = oauth::required_scope(&tag);
        if !oauth::GRANTED_SCOPES.contains(&required) {
            // `warn!`, not `debug!`: deployments run at `RUST_LOG=info`, and a
            // client asking for a method this deployment withholds is the
            // refusal an operator most needs to see.
            warn!(%tag, %required, user_id = %identity.user_id, "T3 UI request refused; scope not granted");
            return vec![typed_failure_exit(
                &id,
                authorization_error(required, format!("This server does not grant {required}.")),
            )];
        }

        match tag.as_str() {
            "server.probe" => vec![success_exit(&id, json!({}))],
            "server.getConfig" => {
                let snapshots = self.published_workspace_snapshots();
                vec![success_exit(
                    &id,
                    server_config(state, identity, &snapshots).await,
                )]
            }
            "server.getSettings" => vec![success_exit(&id, json!({}))],
            // The composer calls this the first time it opens on a `cwd` whose
            // skills it does not have, and reads the answer off the config
            // stream. Both are published: the reply is what stops it asking
            // again, the event is what fills the menu.
            "server.refreshProviders" => {
                if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
                    self.resolve_workspace_skills(state, identity, cwd).await;
                }
                let agents = state.agents.list(&viewer_scope(identity)).await;
                let providers = providers::providers(
                    &agents,
                    &state.model_defaults,
                    &now_rfc3339(),
                    &self.published_workspace_snapshots(),
                );
                let mut frames = vec![success_exit(&id, json!({ "providers": providers.clone() }))];
                if let Some(stream) = self.config_stream.clone() {
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
            // The one write. A refusal is a typed failure the composer renders,
            // not a defect: "that action does not exist here" is true, "this
            // server is broken" is not.
            "orchestration.dispatchCommand" => {
                let ctx = commands::CommandContext {
                    conversations: state.conversations.as_ref(),
                    agents: state.agents.as_ref(),
                    sink: state.commands.as_ref(),
                    workspace_script_commands: state.workspace_script_commands.as_ref(),
                    skills: state.skills.as_ref(),
                    caller: identity.user_id,
                };
                match commands::dispatch(&ctx, &viewer_scope(identity), &payload).await {
                    Ok(result) => vec![success_exit(&id, result)],
                    Err(refusal) => {
                        // `warn!`, not `debug!`: this is the surface's only
                        // write, and at `RUST_LOG=info` a refused one would
                        // otherwise leave nothing behind at all.
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
            "server.getUsageSummary" => {
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
                match state.usage.buckets(&viewer_scope(identity), &window).await {
                    Ok(rows) => {
                        let elapsed =
                            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                        vec![success_exit(
                            &id,
                            usage::summary(&window, &rows, &now_rfc3339(), elapsed),
                        )]
                    }
                    // The cause is logged, not sent: it names tables and
                    // columns, and the client renders `detail` verbatim.
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
            "server.discoverSourceControl" => {
                let github = state
                    .source_control_auth
                    .github_identity(&viewer_scope(identity))
                    .await;
                vec![success_exit(&id, git_actions::discovery(github.as_ref()))]
            }
            "vcs.listRefs" => {
                let Some(cwd) = workspace_cwd(&payload) else {
                    return vec![success_exit(&id, review::no_refs())];
                };
                let request = review::RefRequest::from_payload(&cwd, &payload);
                // A draft thread has no Sandbox yet and addresses its Workspace
                // by the project root. Its branches are the primary
                // repository's, read from the git host — the same list the
                // Moatless branch picker shows before a Task runs.
                if projection::is_project_root(&cwd) {
                    return match review::workspace_branches(
                        state.workspaces.as_ref(),
                        state.branches.as_ref(),
                        &viewer_scope(identity),
                        &cwd,
                        &request,
                    )
                    .await
                    {
                        Ok(refs) => vec![success_exit(&id, refs)],
                        Err(error) => vec![branch_list_exit(&id, &cwd, error, identity)],
                    };
                }
                match review::list_refs(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer_scope(identity),
                    &request,
                )
                .await
                {
                    Ok(refs) => vec![success_exit(&id, refs)],
                    Err(error) if review::refs_are_quiet(&error) => {
                        vec![success_exit(&id, review::no_refs())]
                    }
                    Err(error) => vec![review_failure_exit(&id, &tag, &cwd, &error, identity)],
                }
            }
            "vcs.refreshStatus" => {
                let Some(cwd) = workspace_cwd(&payload) else {
                    // No cwd is why this failed, so there is none to report.
                    return vec![review_failure_exit(
                        &id,
                        &tag,
                        "",
                        &WorkspaceAccessError::Unaddressable,
                        identity,
                    )];
                };
                match review::status_result(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    state.pull_requests.as_ref(),
                    &viewer_scope(identity),
                    &cwd,
                )
                .await
                {
                    Ok(status) => vec![success_exit(&id, status)],
                    Err(error) => vec![review_failure_exit(&id, &tag, &cwd, &error, identity)],
                }
            }
            "vcs.switchRef" => {
                // A draft thread has no checkout to switch: its branch is
                // recorded on the draft and applied when the Task is created,
                // so the switch is answered as done, echoing the ref the
                // client should store. A thread that already has a Sandbox
                // cannot change branch — that is fixed at creation — so the
                // switch there falls through to unsupported.
                match branch_switch_result(&payload) {
                    Some(result) => vec![success_exit(&id, result)],
                    None => vec![unsupported_exit(&id, &tag)],
                }
            }
            "review.getDiffPreview" => {
                let Some(cwd) = workspace_cwd(&payload) else {
                    // No cwd is why this failed, so there is none to report.
                    return vec![review_failure_exit(
                        &id,
                        &tag,
                        "",
                        &WorkspaceAccessError::Unaddressable,
                        identity,
                    )];
                };
                match review::diff_preview(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer_scope(identity),
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
                    Err(error) => vec![review_failure_exit(&id, &tag, &cwd, &error, identity)],
                }
            }
            "terminal.open" => {
                let viewer = viewer_scope(identity);
                match terminal::open(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &state.terminal,
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(snapshot) => vec![success_exit(&id, snapshot)],
                    Err(error) => vec![terminal_failure_exit(&id, &payload, &error, identity)],
                }
            }
            "terminal.attach" => {
                let viewer = viewer_scope(identity);
                let stream = match terminal::attach(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(stream) => stream,
                    Err(error) => {
                        return vec![terminal_failure_exit(&id, &payload, &error, identity)];
                    }
                };
                let terminal = state.terminal.clone();
                self.streams.start_produced(&id, None, move |sink| {
                    terminal::forward_attach_stream(stream, terminal, sink)
                })
            }
            "terminal.write" => {
                let viewer = viewer_scope(identity);
                match terminal::write(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(()) => vec![success_exit(&id, Value::Null)],
                    Err(error) => vec![terminal_failure_exit(&id, &payload, &error, identity)],
                }
            }
            "terminal.resize" => {
                let viewer = viewer_scope(identity);
                match terminal::resize(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(()) => vec![success_exit(&id, Value::Null)],
                    Err(error) => vec![terminal_failure_exit(&id, &payload, &error, identity)],
                }
            }
            "terminal.clear" => {
                let viewer = viewer_scope(identity);
                match terminal::clear(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &state.terminal,
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(()) => vec![success_exit(&id, Value::Null)],
                    Err(error) => vec![terminal_failure_exit(&id, &payload, &error, identity)],
                }
            }
            "terminal.restart" => {
                let viewer = viewer_scope(identity);
                match terminal::restart(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &state.terminal,
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(snapshot) => vec![success_exit(&id, snapshot)],
                    Err(error) => vec![terminal_failure_exit(&id, &payload, &error, identity)],
                }
            }
            "terminal.close" => {
                let viewer = viewer_scope(identity);
                match terminal::close(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &state.terminal,
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(()) => vec![success_exit(&id, Value::Null)],
                    Err(error) => vec![terminal_failure_exit(&id, &payload, &error, identity)],
                }
            }
            "preview.list" => preview_answer(
                &id,
                preview::list(&state.preview, &viewer_scope(identity), &payload).await,
            ),
            "preview.open" => preview_answer(
                &id,
                preview::open(&state.preview, &viewer_scope(identity), &payload).await,
            ),
            "preview.navigate" => preview_answer(
                &id,
                preview::navigate(&state.preview, &viewer_scope(identity), &payload).await,
            ),
            "preview.resize" => preview_answer(
                &id,
                preview::resize(&state.preview, &viewer_scope(identity), &payload).await,
            ),
            // The three void methods answer `null` rather than a snapshot,
            // matching the contract's declared success type.
            "preview.refresh" => preview_answer(
                &id,
                preview::refresh(&state.preview, &viewer_scope(identity), &payload)
                    .await
                    .map(|()| Value::Null),
            ),
            "preview.reportStatus" => preview_answer(
                &id,
                preview::report_status(&state.preview, &viewer_scope(identity), &payload)
                    .await
                    .map(|()| Value::Null),
            ),
            "preview.close" => preview_answer(
                &id,
                preview::close(&state.preview, &viewer_scope(identity), &payload)
                    .await
                    .map(|()| Value::Null),
            ),
            "servers.list" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(&id, "servers.list requires a threadId")];
                };
                match servers::list(&state.servers, &viewer, task_id).await {
                    Ok(listing) => vec![success_exit(&id, listing)],
                    // See [`servers_not_found`]: an unprovisioned conversation
                    // has no servers, which is an answer and not a failure.
                    Err(ServersError::NotFound) => vec![success_exit(&id, servers_not_found())],
                    Err(error) => vec![servers_failure_exit(&id, &error, identity)],
                }
            }
            "subtasks.list" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(&id, "subtasks.list requires a threadId")];
                };
                match subtasks::list(&state.subtasks, &viewer, task_id).await {
                    Ok(listing) => vec![success_exit(&id, listing)],
                    // A Thread the viewer cannot read has no subtasks *for
                    // them*, and the panel is a section on a page they are
                    // already looking at: an empty list is the honest answer
                    // and a failure would red-flag a working page.
                    Err(SubtasksError::NotFound) => {
                        vec![success_exit(&id, json!({ "subtasks": [] }))]
                    }
                    Err(error) => vec![subtasks_failure_exit(&id, &error, identity)],
                }
            }
            "scripts.run" => {
                let viewer = viewer_scope(identity);
                match scripts::run(
                    &state.scripts,
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &state.terminal,
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(result) => vec![success_exit(&id, result)],
                    Err(error) => vec![scripts_failure_exit(&id, &error, &payload, identity)],
                }
            }
            "sandbox.status" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(&id, "sandbox.status requires a threadId")];
                };
                match sandbox::status(&state.sandbox, &viewer, task_id).await {
                    Ok(status) => vec![success_exit(&id, status)],
                    Err(error) => vec![sandbox_failure_exit(&id, &tag, &error, identity)],
                }
            }
            "sandbox.start" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(&id, "sandbox.start requires a threadId")];
                };
                match sandbox::start(&state.sandbox, &viewer, task_id).await {
                    Ok(status) => vec![success_exit(&id, status)],
                    Err(error) => vec![sandbox_failure_exit(&id, &tag, &error, identity)],
                }
            }
            "sandbox.stop" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(&id, "sandbox.stop requires a threadId")];
                };
                match sandbox::stop(&state.sandbox, &viewer, task_id).await {
                    Ok(status) => vec![success_exit(&id, status)],
                    Err(error) => vec![sandbox_failure_exit(&id, &tag, &error, identity)],
                }
            }
            "sandbox.subscribeStatus" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(
                        &id,
                        "sandbox.subscribeStatus requires a threadId",
                    )];
                };
                let task_id = task_id.to_string();
                // Seed from the same read the poller makes, so the indicator
                // has a status before the first interval elapses rather than
                // after it.
                let seeded = match sandbox::status(&state.sandbox, &viewer, &task_id).await {
                    Ok(status) => status,
                    Err(error) => return vec![sandbox_failure_exit(&id, &tag, &error, identity)],
                };
                let sandbox = state.sandbox.clone();
                self.streams
                    .start_produced(&id, Some(vec![seeded.clone()]), move |sink| {
                        sandbox::watch_status(sink, sandbox, viewer, task_id, seeded)
                    })
            }
            "sandbox.listDiagnostics" => match sandbox::list_diagnostics(&state.sandbox).await {
                Ok(entries) => vec![success_exit(&id, entries)],
                Err(error) => vec![sandbox_failure_exit(&id, &tag, &error, identity)],
            },
            "subscribeServerStatus" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(
                        &id,
                        "subscribeServerStatus requires a threadId",
                    )];
                };
                let task_id = task_id.to_string();
                // Seed from the same read the poller makes, so the panel has a
                // list before the first interval elapses rather than after it.
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
                    Err(error) => return vec![servers_failure_exit(&id, &error, identity)],
                };
                let servers = state.servers.clone();
                self.streams
                    .start_produced(&id, Some(vec![seeded]), move |sink| {
                        servers::watch_status(sink, servers, viewer, task_id)
                    })
            }
            "servers.subscribeLogs" => {
                let viewer = viewer_scope(identity);
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
            "projects.listEntries" => {
                let Some(cwd) = workspace_cwd(&payload) else {
                    return vec![file_failure_exit(
                        &id,
                        &tag,
                        &WorkspaceAccessError::Unaddressable,
                        identity,
                    )];
                };
                match files::list_entries(
                    state.conversations.as_ref(),
                    state.file_sources(),
                    &viewer_scope(identity),
                    &cwd,
                )
                .await
                {
                    Ok(listing) => vec![success_exit(&id, listing)],
                    Err(error) => vec![file_failure_exit(&id, &tag, &error, identity)],
                }
            }
            "projects.readFile" => {
                let relative_path = payload
                    .get("relativePath")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|path| !path.is_empty());
                let (Some(cwd), Some(relative_path)) = (workspace_cwd(&payload), relative_path)
                else {
                    return vec![file_failure_exit(
                        &id,
                        &tag,
                        &WorkspaceAccessError::Unaddressable,
                        identity,
                    )];
                };
                match files::read_file(
                    state.conversations.as_ref(),
                    state.file_sources(),
                    &viewer_scope(identity),
                    &cwd,
                    relative_path,
                )
                .await
                {
                    Ok(file) => vec![success_exit(&id, file)],
                    Err(error) => vec![file_failure_exit(&id, &tag, &error, identity)],
                }
            }
            "projects.searchEntries" => {
                // An absent or blank query is legal and means "show me what is
                // here": the picker opens before anyone types. `search_entries`
                // answers that with a bounded listing rather than nothing,
                // because the Sandbox's name search matches no path at all for
                // an empty needle and an empty picker looks like a broken one.
                let query = payload
                    .get("query")
                    .and_then(Value::as_str)
                    .map_or("", str::trim);
                let Some(cwd) = workspace_cwd(&payload) else {
                    return vec![file_failure_exit(
                        &id,
                        &tag,
                        &WorkspaceAccessError::Unaddressable,
                        identity,
                    )];
                };
                let limit = payload
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map_or(SEARCH_ENTRIES_LIMIT, |limit| {
                        (limit as usize).min(SEARCH_ENTRIES_MAX)
                    });
                match files::search_entries(
                    state.conversations.as_ref(),
                    state.file_sources(),
                    &viewer_scope(identity),
                    &cwd,
                    query,
                    limit,
                    payload.get("kind").and_then(Value::as_str),
                )
                .await
                {
                    Ok(entries) => vec![success_exit(&id, entries)],
                    Err(error) => vec![file_failure_exit(&id, &tag, &error, identity)],
                }
            }
            "projects.writeFile" => {
                let relative_path = payload
                    .get("relativePath")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|path| !path.is_empty());
                // Empty contents are a legal write — truncating a file is a
                // thing a person does — so only an absent `contents` is
                // unaddressable.
                let contents = payload.get("contents").and_then(Value::as_str);
                let (Some(cwd), Some(relative_path), Some(contents)) =
                    (workspace_cwd(&payload), relative_path, contents)
                else {
                    return vec![file_failure_exit(
                        &id,
                        &tag,
                        &WorkspaceAccessError::Unaddressable,
                        identity,
                    )];
                };
                match files::write_file(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer_scope(identity),
                    &cwd,
                    relative_path,
                    contents,
                )
                .await
                {
                    Ok(written) => vec![success_exit(&id, written)],
                    Err(error) => vec![file_failure_exit(&id, &tag, &error, identity)],
                }
            }
            "orchestration.searchThreads" => {
                let query = payload
                    .get("query")
                    .and_then(Value::as_str)
                    .map_or("", str::trim);
                // See [`SEARCH_THREADS_MIN_QUERY`]. Answering an empty list is
                // the honest response to "match anything", and it costs nothing.
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
                    .conversations
                    .search_conversations(&viewer_scope(identity), query, limit)
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
            "cloud.getRelayClientStatus" => vec![success_exit(
                &id,
                json!({
                    "status": "unsupported",
                    "platform": platform_os(),
                    "arch": platform_arch(),
                    "version": T3_CLIENT_CONTRACT_VERSION
                }),
            )],
            // The listings' complement: one row, by id, for a conversation the
            // client is holding and no listing named. `null` is the answer for
            // every way it can be absent — the client is already on that page,
            // and a failure would take a rendered transcript off the screen to
            // report a missing title.
            "threads.getShell" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(&id, "threads.getShell requires a threadId")];
                };
                match thread_shell(
                    state.workspaces.as_ref(),
                    state.conversations.as_ref(),
                    &state.model_defaults,
                    &viewer,
                    task_id,
                )
                .await
                {
                    Ok(thread) => vec![success_exit(
                        &id,
                        json!({ "thread": thread.unwrap_or(Value::Null) }),
                    )],
                    Err(error) => vec![shell_failure_exit(&id, &error, identity)],
                }
            }
            "orchestration.getArchivedShellSnapshot" => {
                let viewer = viewer_scope(identity);
                match archived_shell_snapshot(
                    state.workspaces.as_ref(),
                    state.conversations.as_ref(),
                    &state.model_defaults,
                    &viewer,
                )
                .await
                {
                    Ok(snapshot) => vec![success_exit(&id, snapshot)],
                    Err(error) => vec![shell_failure_exit(&id, &error, identity)],
                }
            }
            // Still answers with nothing, and deliberately. This is not the
            // path a reconnecting client takes: no T3 client calls it — web,
            // desktop and mobile all share `client-runtime`, whose only resume
            // is `subscribeShell` with `afterSequence` (`state/shell.ts:181`),
            // and `replayEvents` appears outside `packages/contracts` only in
            // the reference server and its own tests. Its result type is
            // `OrchestrationEvent`, which carries an event id, an aggregate, a
            // causation chain and a typed payload; a listing change record
            // carries a position and a Task id and nothing else, on purpose.
            // Filling this in would mean inventing an event log this backend
            // does not have, for a method nothing calls.
            "orchestration.replayEvents" => vec![success_exit(&id, json!([]))],
            "orchestration.subscribeShell" => {
                let viewer = viewer_scope(identity);
                let subscription = match shell_subscription(
                    state.workspaces.as_ref(),
                    state.conversations.as_ref(),
                    &state.model_defaults,
                    &viewer,
                    &state.environment,
                    after_sequence(&payload),
                )
                .await
                {
                    Ok(subscription) => subscription,
                    Err(error) => return vec![shell_failure_exit(&id, &error, identity)],
                };
                // Both halves come from `shell_subscription`: the items to send,
                // and a feed that already knows what sending them leaves the
                // client holding.
                let mut values = subscription.values;
                if payload
                    .get("requestCompletionMarker")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    // Last, after the snapshot or the catch-up. The contract
                    // places the marker "after the subscription has emitted its
                    // initial snapshot or catch-up replay and before it begins
                    // emitting live events", and the client reads it as
                    // permission to stop rendering itself as synchronizing.
                    values.push(json!({ "kind": "synchronized" }));
                }
                let workspaces = state.workspaces.clone();
                let conversations = state.conversations.clone();
                let model_defaults = state.model_defaults.clone();
                self.streams.start_produced(&id, Some(values), move |sink| {
                    watch::watch_shell(
                        sink,
                        subscription.feed,
                        workspaces,
                        conversations,
                        model_defaults,
                        viewer,
                    )
                })
            }
            "orchestration.subscribeThread" => {
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    // The contract requires it and every client encodes against
                    // the contract, so this is a malformed request rather than
                    // a conversation that is missing.
                    return vec![failure_exit(
                        &id,
                        "orchestration.subscribeThread requires a threadId",
                    )];
                };
                let task_id = task_id.to_string();
                let viewer = viewer_scope(identity);
                let opened = match thread_subscription(
                    state.workspaces.as_ref(),
                    state.conversations.as_ref(),
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
                let conversations = state.conversations.clone();
                let frontend_url = state.frontend_url.clone();
                self.streams.start_produced(&id, Some(values), move |sink| {
                    watch::watch_thread(sink, feed, conversations, frontend_url, viewer)
                })
            }
            // A conversation's checkout exists inside its Sandbox and nowhere
            // else, so an asset is minted as a route this backend serves the
            // file on rather than as a path on this machine — see [`assets`].
            // `workspace-file` and `media-file` both name a file in that
            // checkout. An attachment is an upload and is minted at the upload
            // route. A project's favicon names no file here and keeps answering
            // that there is none, which is what the client renders as a project
            // without an icon.
            "assets.createUrl" => {
                // The error carries the resource back, and the client decodes it
                // against the same schema it sent — so a request without one has
                // no typed answer to give and is a malformed request instead.
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
            // Where the composer sends a file before it sends the turn naming
            // it. The URL carries its own credential and the route serves it
            // — see [`attachments`] — so nothing about the upload reaches this
            // socket except the id it will be known by.
            "attachments.createUploadUrl" => {
                match attachments::create_upload_url(
                    identity,
                    state.session_jwt_secret.as_deref(),
                    &payload,
                ) {
                    Ok(minted) => vec![success_exit(&id, minted)],
                    Err(attachments::MintError::Invalid(message)) => {
                        vec![failure_exit(&id, message)]
                    }
                    Err(attachments::MintError::NoSigningKey) => vec![typed_failure_exit(
                        &id,
                        json!({
                            "_tag": "AttachmentUploadSigningKeyError",
                            "cause": "This deployment cannot sign attachment upload URLs."
                        }),
                    )],
                }
            }
            "attachments.delete" => {
                match attachments::delete(
                    state.attachments.as_ref(),
                    &viewer_scope(identity),
                    &payload,
                )
                .await
                {
                    Ok(()) => vec![success_exit(&id, Value::Null)],
                    Err(message) => vec![failure_exit(&id, message)],
                }
            }
            // Which client is focused decides whether *T3* keeps background
            // work running on its own host. Moatless runs its agents in
            // Sandboxes that no browser tab is attached to, so the report is
            // accepted and dropped rather than refused: the client sends it on
            // every focus change and a defect per change is pure noise.
            "server.reportClientActivity" => vec![success_exit(&id, Value::Null)],
            "subscribeServerConfig" => {
                let snapshot = json!({
                    "version": 1,
                    "type": "snapshot",
                    "config": server_config(
                        state,
                        identity,
                        &self.published_workspace_snapshots(),
                    ).await
                });
                // A client that did not ask for `usageLimitSourcesUpdated`
                // decodes the stream against an event union without it and dies
                // on one, so the opt-in gates the whole producer rather than
                // only the first frame.
                let sources_opted_in = state.rpc.usage_limit_sources
                    && payload
                        .get("usageLimitSources")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                let started = if sources_opted_in {
                    let sources = state.usage_limit_sources.clone();
                    let mut changes = sources.subscribe();
                    let first = sources_event(sources.snapshot().await);
                    self.streams.start_produced(
                        &id,
                        Some(vec![snapshot, first]),
                        move |sink| async move {
                            loop {
                                match changes.recv().await {
                                    Ok(next) => {
                                        if !sink.send(sources_event(next)).await {
                                            return Ok(());
                                        }
                                    }
                                    // The next tick republishes the whole set,
                                    // so a dropped one costs a minute of
                                    // staleness and nothing else.
                                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                                }
                            }
                        },
                    )
                } else {
                    self.streams.start(&id, Some(vec![snapshot]))
                };
                if self.streams.contains(id.as_str()) {
                    self.config_stream = Some(id.clone());
                }
                started
            }
            "subscribeServerLifecycle" => self.streams.start(
                &id,
                Some(vec![json!({
                    "version": 1,
                    "sequence": 1,
                    "type": "welcome",
                    "payload": {
                        "environment": descriptor(&state.rpc),
                        "cwd": "/moatless",
                        "projectName": T3_ENVIRONMENT_LABEL
                    }
                })]),
            ),
            "subscribeAuthAccess" => self.streams.start(
                &id,
                Some(vec![json!({
                    "version": 1,
                    "revision": 1,
                    "type": "snapshot",
                    "payload": {
                        "pairingLinks": [],
                        "clientSessions": []
                    }
                })]),
            ),
            "subscribePreviewEvents" => {
                let preview = state.preview.clone();
                let viewer = viewer_scope(identity);
                self.streams.start_produced(&id, None, move |sink| {
                    watch::watch_preview_events(sink, preview, viewer)
                })
            }
            "subscribeTerminalEvents" => {
                let events = state.terminal.subscribe_events();
                self.streams.start_produced(&id, None, move |sink| {
                    terminal::forward_value_subscription(events, sink)
                })
            }
            "subscribeTerminalMetadata" => {
                let snapshot = state.terminal.metadata_snapshot().await;
                let metadata = state.terminal.subscribe_metadata();
                self.streams
                    .start_produced(&id, Some(vec![snapshot]), move |sink| {
                        terminal::forward_value_subscription(metadata, sink)
                    })
            }
            "subscribeVcsStatus" => {
                // A `cwd` this surface does not address is not a failure here:
                // the client opens this subscription for every thread it shows,
                // including ones with no sandbox, and a stream that says nothing
                // is what "there is no repository to report on" looks like.
                let Some(cwd) = workspace_cwd(&payload) else {
                    return self.streams.start(&id, None);
                };
                let conversations = state.conversations.clone();
                let sandboxes = state.sandboxes.clone();
                let pull_requests = state.pull_requests.clone();
                let viewer = viewer_scope(identity);
                self.streams.start_produced(&id, None, move |sink| {
                    watch::watch_vcs_status(
                        sink,
                        cwd,
                        conversations,
                        sandboxes,
                        pull_requests,
                        viewer,
                    )
                })
            }
            "git.runStackedAction" => {
                // The action is not run here or in the Sandbox: the button press
                // becomes a message asking the Task's agent to do the git work.
                // The client's contract is a stream, so it is answered with one
                // `action_finished` chunk and a terminal success, the same shape
                // a completed action produced.
                let viewer = viewer_scope(identity);
                match git_actions::run_stacked_action(
                    state.conversations.as_ref(),
                    state.commands.as_ref(),
                    &viewer,
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
                    Err(error) => vec![git_action_failure_exit(&id, &error, identity)],
                }
            }
            // The one method of the `pullRequests.*` group this backend serves.
            // The rest of the group reads a git host and is refused, which is
            // why `capabilities.pullRequests` stays absent: the listing route,
            // the right-panel surface and the review actions all gate on it,
            // and none of them can be served from a binding.
            "pullRequests.summary" => {
                let Some((project_id, repository, number)) = pull_request_ref(&payload) else {
                    return vec![pull_request_summary_failure_exit(
                        &id,
                        "A pull request reference needs a projectId, repository and number.",
                    )];
                };
                match review::pull_request_summary(
                    state.conversations.as_ref(),
                    state.pull_requests.as_ref(),
                    &viewer_scope(identity),
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
            "git.resolvePullRequest" => {
                match git_actions::resolve_pull_request(
                    state.workspaces.as_ref(),
                    state.conversations.as_ref(),
                    state.pull_requests.as_ref(),
                    &viewer_scope(identity),
                    &payload,
                )
                .await
                {
                    Ok(result) => vec![success_exit(&id, result)],
                    Err(error) => vec![pull_request_failure_exit(&id, &tag, &error, identity)],
                }
            }
            "git.preparePullRequestThread" => {
                match git_actions::prepare_pull_request_thread(
                    state.workspaces.as_ref(),
                    state.conversations.as_ref(),
                    state.pull_requests.as_ref(),
                    &viewer_scope(identity),
                    &payload,
                )
                .await
                {
                    Ok(git_actions::PreparedThread::Local(result)) => {
                        vec![success_exit(&id, result)]
                    }
                    // Worktree mode is refused by name, not by falling through to
                    // the catch-all: this surface prepares a branch, never a
                    // worktree on disk.
                    Ok(git_actions::PreparedThread::WorktreeUnsupported) => {
                        vec![unsupported_exit(&id, &tag)]
                    }
                    Err(error) => vec![pull_request_failure_exit(&id, &tag, &error, identity)],
                }
            }
            // `subscribeDiscoveredLocalServers` falls through here and says so:
            // there is no local-server discovery, and a stream that never says
            // anything reads to a client as "connected, nothing to report".
            _ => vec![unsupported_exit(&id, &tag)],
        }
    }
}
