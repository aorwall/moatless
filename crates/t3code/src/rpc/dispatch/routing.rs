//! The two matches that name every method this surface serves. Neither holds
//! a method body: one function per method, named for its tag, snake cased —
//! `pullRequests.summary` is `pull_requests_summary`.
//!
//! Detach a read; keep a transition on the loop. A method that moves a named
//! resource from one state to another — `terminal.*`, `projects.writeFile`,
//! `sandbox.start`/`stop`, `scripts.run`, `orchestration.dispatchCommand`,
//! `attachments.delete`, `git.runStackedAction`, the mutating `preview.*` —
//! stays in [`ProtocolSession::handle_request`]. The loop is sequential, so a
//! transition on it finishes before anything the client sent after it is
//! dispatched; that is what a detached read is allowed to rely on, and
//! detaching a transition would take it away.

use super::orchestration::*;
use super::preview::*;
use super::projects::*;
use super::sandbox::*;
use super::server::*;
use super::terminal::*;
use super::vcs::*;
use crate::oauth;
use crate::rpc::frames::RequestId;
use crate::rpc::frames::authorization_error;
use crate::rpc::frames::connection_defect;
use crate::rpc::frames::typed_failure_exit;
use crate::rpc::frames::unsupported_exit;
use crate::rpc::frames::valid_request_id;
use crate::rpc::metrics::answered;
use crate::rpc::session::ProtocolSession;
use crate::rpc::session::SharedSession;
use crate::state::UiRpcState;
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use serde_json::Value;
use std::future::Future;
use std::future::ready;
use std::pin::Pin;
use tracing::warn;

/// One client request, owned by whatever answers it.
///
/// Owned rather than borrowed because a detached request outlives the frame-loop
/// call that routed it. Every field is an `Arc`, a short `String` or the `Value`
/// the frame already parsed, so building one costs a set of refcount bumps.
pub(super) struct Request {
    pub(super) id: RequestId,
    /// The method name, kept because the failure frames echo it back.
    pub(super) tag: String,
    pub(super) payload: Value,
    pub(super) state: UiRpcState,
    pub(super) identity: UiRpcAuthIdentity,
    pub(super) shared: SharedSession,
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

        // The deployment's grant, not the client's: a pairing that asked for
        // less than this still gets a credential carrying all of it.
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
/// tag absent from it reaches [`ProtocolSession::handle_request`]. See the
/// module doc for which methods belong here.
///
/// A detached read may observe a transition the client sent *after* it, and
/// answers in no particular order against the requests around it.
///
/// A detached method may not touch [`ProtocolSession`]: each one is a free
/// function taking an owned [`Request`], and the compiler enforces it. State a
/// detached method genuinely needs goes in `SharedSession` (`rpc/session.rs`)
/// behind its own lock, held for a single read or write and never across an
/// await.
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
