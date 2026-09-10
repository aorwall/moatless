# Deferred changes

Changes we found while doing something else, decided not to make at the time, and
do not want to rediscover. Every entry names the condition that makes it worth
doing — an entry with no condition is an opinion, not a deferral, and does not
belong here.

This is not a backlog of ideas and not a place to park work nobody has looked at.
Each entry below was reached by reading the code it points at.

## Working rules

The repository has deleted unmaintained markdown before. These rules are what
keep this file from becoming that:

- **Anchor by file and symbol, not line number.** Line numbers rot in a week;
  `workspace/handlers.rs::add_workspace_repo` survives a refactor.
- **Delete an entry when it is done.** Do not mark it done. `git log` is the
  history; a file of crossed-out entries is noise.
- **Delete an entry whose anchor no longer exists.** If the code moved and the
  concern survived, re-anchor it in the same change. If the concern went away
  with the code, it is gone — say nothing.
- **Link reasoning, never copy it.** Where another document owns the argument,
  point at it. Two copies disagree eventually and the reader cannot tell which
  one is current.

## Layering — the ports-and-crates program

`.plans/backend-ports-and-crates.md` is authoritative for all of these: it holds
the argument, the pricing, and the step each one blocks. Listed here only so a
reader outside that program can see them.

| Deferred | Anchor | Entry condition |
|---|---|---|
| `AgentErrorSink` has one production implementation | `task/reaper.rs::AgentErrorSink`, implemented for `AppState` in `event_handler/agent_handlers.rs` | Step 10d — see below |
| The reaper's invocation of the sink is untested | `task/reaper.rs::close_orphaned_turn` calls `close_orphaned_turn_as_error`; only the helper is covered | Step 10d, when `TaskReaper` becomes constructible from fakes |
| `SandboxWakeService`'s closed-task guard is untested | `crates/sandbox-control/src/notify.rs::SandboxWakeService::wake` — the "a closed task does not wake" branch; only the seam (`moatless_tasks::message::wake::wake_for_message`) is covered, not the wake behaviour | When the rich `Lifecycle` is behind a trait and `SandboxWakeService` is constructible from fakes — the guard now reads `Lifecycle::intent` and the start calls `Lifecycle::start`, so one trait covers both |
| `TaskReaper`'s two above-band concrete fields | `task/reaper.rs::TaskReaper::new` — `k8s: K8sClient`, `sandbox: Arc<Lifecycle>` | Step 10d — a precondition of the crate compiling, not a cleanup |
| `sync_config`'s body spans two bands | `repository/handlers.rs::sync_repo_config` runs a loop sync and a workspace sync | Step 10g, as a server-band composition |
| The error-type knot | `error.rs` split, 7c's registry inversion, 8b's remainder, 8d's error half | One knot, one entry condition, recorded in the plan |
| `SetupRequirementCheck` is a single-implementation trait | `setup/evaluator.rs::SetupRequirementCheck` — `checks()` returns `vec![Box::new(GithubTokenRequirement)]`; recorded as 8h | When it earns a second implementation, or is collapsed into `evaluate_setup` |
| `moatless-api-schemas` is one crate for every domain | `crates/api-schemas/src/` — 15 crates depend on it, and each consumer already has a `<domain>/schemas.rs` that is one `pub use moatless_api_schemas::<domain>::*;` | When seven or eight more crates is an accepted cost. It is the single largest rebuild-cost edge in the workspace and the split itself is mechanical; the crate count is the decision |

**On `AgentErrorSink` specifically.** It is a consumer-owned port with exactly one
production implementation, created so the reaper stops naming `AppState` and comes
off `tooling/arch-rules/arch-baseline.json`. It clears the repository's
"no trait with a single implementation" bar only because a test uses the recording
mock. That is a real cost paid for a real edge, but it is worth revisiting rather
than treating as settled: once 10d inverts `k8s` and `sandbox` to ports, the
reaper's dependencies are uniform, and the question becomes whether the sink is
still the right shape or whether the error-close path should move instead. Decide
it then, with the whole constructor visible, rather than defending the port
because it exists.

**`SandboxWaker` is a different case and is settled, so it carries no row.** It is
also a consumer-owned port with one production implementation, but it is not held
up by a test fake: it is a crate boundary. Its body needs `Lifecycle` and
`NotifyPort` (sandbox-control, band 6) while every caller sits at band 5 or above,
so no placement of the code makes the reference point downward — the trait is the
only thing that does. Collapsing it back into a function restores a `5 → 6` edge
and, at the loops-band call site, restores `loops`' dependency on
`sandbox-control` outright. It is declared in `moatless-ports` beside the
other sandbox ports, which is what lets `tasks` and `sandbox-control` be siblings.
The reasoning, its discriminator against `SetupRequirementCheck`, and the
assertions that pin it are in `.plans/backend-ports-and-crates.md` under Step 5.

## Layering — types coupled to a transport crate

`moatless-types` holds the shared type definitions. Its `lib.rs` states the rule
that admits a type: pure data, named by three or more crates other than its
owner, and neither its body nor its impls name a foreign crate. The crate depends
on `serde`, `serde_json`, `chrono`, `thiserror`, `tracing`, `url`, `uuid`, and
optionally `utoipa`, and on nothing else — no `sqlx`, no `axum`, no `reqwest`,
no `redis`.
That last constraint is what keeps a change to a transport crate from rebuilding
everything above it.

Each type below fails the third clause: one variant or one body reaches a
transport crate, so moving it would put that crate in `moatless-types`. The
fan-in bar is counted per row in the entry condition — a type stops naming a
transport long before three crates outside its owner name it, and the two are
independent gates rather than one.

| Deferred | Anchor | Entry condition |
|---|---|---|
| `SharedAdapterError` names a transport | `crates/adapter-model/src/error.rs::SharedAdapterError` — `ProviderDecodeError::Body(#[from] reqwest::Error)` and the two `reqwest`-carrying provider variants | When the transport-level variants are split off the domain error, so the domain half names no transport |
| `TaskCreationError` names the store | `crates/tasks/src/task/creation.rs::TaskCreationError` — `Db(#[from] sqlx::Error)`, beside a `Store(#[from] StoreError)` variant that already carries the seam half | When a crate outside `tasks` names it. None does today, so the type is that crate's own vocabulary and the driver in `Db` is what a caller inside the crate already compiles |
| `CredentialError` names the store | `crates/identity/src/credentials/error.rs::CredentialError` — `Database(#[from] sqlx::Error)` | Ready now: `backend`, `tasks` and `workspaces` all name it, which is the third crate the bar asks for. Splitting `Database` off changes the `Display` text those call sites map on, so it is its own change with its own review |

A type named by only one or two crates other than its owner is that subsystem's
vocabulary and stays with the subsystem — `sandbox-control`'s lifecycle types go
to `sandbox-k8s` and `sandbox-docker` and nowhere else. Row types stay with their
DAO for the same reason the four above stay put: a `sqlx::FromRow` derive is a
persistence detail, not a shared definition.

## Domain models still carrying a persistence shape

Each entity below has its domain model separated from its row, except for the
one field named. The fix is known in every case; what stops it is stated.

| Deferred | Anchor | Entry condition |
|---|---|---|
| A Message's timestamp is text on the wire | `crates/tasks/src/message/models.rs::TaskMessage::timestamp` — a `text` column, `ORDER BY m.timestamp, m.id`, and the map key in `message/transform.rs` | When the frontend is confirmed to accept `to_rfc3339()`'s `+00:00` form in place of the stored `…Z` with milliseconds, and the ordering key is proven stable across the change |
| A Message's agent mode is read through an accessor | `crates/tasks/src/message/models.rs::TaskMessage::agent_mode_raw` and `TaskMessage::agent_mode` | When `TaskMessage` needs a row twin for another reason — the sqlx derive cannot decode `Option<AgentMode>` (the orphan rule blocks `impl TryFrom<Option<String>> for Option<AgentMode>`), so today the fix costs a fifty-field mapping for one field |
| A Binding's origin is an open string | `crates/types/src/adapter.rs::TaskBinding::origin`, against `ADAPTER_EVENT_BINDING_ORIGIN` and its two siblings in `crates/adapter-model/src/binding.rs` | When the legal set is settled: the three declared constants are not the whole set — `bindings/handlers.rs` writes `"api"` and the Linear delivery tests write `"provider_event"` |
| A Binding's `subject_kind` and a connection's `connection_kind` are open strings | `crates/types/src/adapter.rs::TaskBinding::subject_kind`, `AdapterConnection::connection_kind` | Same as `origin`: each Adapter declares its own values and no enumeration exists |
| A Task's Workspace is optional in a model where every Task has one | `crates/types/src/task.rs::Task::workspace_id` — `Option<String>` over the nullable `tasks.workspace_id` (`crates/core/migrations/0079_workspaces.sql`), while `crates/tasks/src/task/dao.rs::insert_task` refuses a Task without one. Every `None` reader is dead for rows written since | When the rows predating Workspaces are backfilled or closed out, so the column takes `NOT NULL` and a foreign key to `workspaces` and the field becomes `String` |
| `User` and `TeamMember` are not in `moatless-types` | `crates/identity/src/auth/users.rs::User`, `crates/identity/src/team/models.rs::TeamMember` — both key on `uuid::Uuid` | `uuid` is now on the list, so the block is gone. Move each one when a third crate outside `identity` names it |

## Table ownership

`tooling/arch-rules/src/table_ownership.rs` fails on a production SQL statement naming a
table another crate owns, against its ownership map. The map must
cover the schema — the checker replays `crates/core/migrations` and fails on a
live table with no owner, and on an owner for a table that no longer exists.
`tooling/arch-rules/arch-baseline.json` holds the statements that predate the
check and only shrinks. Deferred decisions sit outside that
file.

| Deferred | Anchor | Entry condition |
|---|---|---|
| Four tables are live but nothing reads or writes them | `crates/core/migrations/0001_schema.sql` creates `task_message_content_blocks`; `0111_github_apps.sql` creates `github_apps`, `github_app_audit_events` and `github_app_migrations`, which `0117_adapter_apps.sql` superseded with the `adapter_app*` tables without dropping. No crate issues SQL against any of the four. They carry an owner in the map so coverage holds — the owner is where the SQL would belong, not evidence of a caller | When a migration is being written against that area anyway. Dropping four tables is a schema migration with a rollback story, and it wants to ride along with a change that already has one rather than be a migration of its own |
| `t3code` reads fifteen tables it does not own | `crates/t3code/src/dao.rs`, `listing/dao.rs`, and `usage/dao.rs` — read projections across `tasks`, `task_messages`, `task_turns`, `task_activities`, `task_follows`, `task_bindings`, `task_message_usage`, `task_message_contexts`, `workspaces`, `workspace_repos`, `users`, and the four listing/conversation change-feed tables. It is now declared a read-only projection in `crates/t3code/AGENTS.md`, which is what the table-ownership exclusion rests on | When one of these reads has to become a write, or the T3 contract changes shape. Either breaks the declaration and forces the reads through the ports |
| `adapter-telegram` pairing is mostly unreachable | `crates/adapter-telegram/src/pairing/` — `dao.rs::find_by_code`, `dao.rs::list_pending`, `dao.rs::prune_expired`, `approval.rs::list_pending_for_user` and `approval.rs::approve_request` have no caller anywhere in the workspace; only `request.rs::upsert_pairing_request` is reached (from `ingress.rs`) | When the Telegram pairing flow is next worked on. Deleting them now would be a behaviour question dressed as a cleanup — nothing says whether the flow is unfinished or abandoned |


## Correctness

Defects found by reading, each real, none urgent enough to have been fixed in the
change that found it.

| Deferred | Anchor | Entry condition |
|---|---|---|
| A restart that dies between taking the unreported commands and queueing the wake loses them from both copies | `crates/sandbox/src/agent/restart.rs::open_for_work` — `take_lost_commands` clears the journal and `sandbox_unreported_commands` before `report_wake_notes` queues anything, so a pod replaced inside that window has consumed both mirrors and announced nothing. The window is small (two reads, then one RPC) and predates the row mirror: the journal's take has always worked this way | When a replacement is observed losing a wake this way, or when the take is being changed for another reason. The fix is to clear on acknowledgement rather than on read — the take returns the commands and a token, and the wake's successful enqueue redeems it — which turns one RPC into two and needs a sweep for tokens no pod ever redeems |
| Six clearable PATCH fields cannot be cleared over HTTP | `crates/api-schemas/src/loops.rs` — `UpdateLoopConfig`'s `ongoing_task_id`, `task_name`, `prompt`, `inactive_ttl_days` and `UpdateSubscription`'s `source_name`, `adapter_config`. Each is `Option<Option<T>>` with the derived deserializer, which collapses an absent key and an explicit `null` to `None`, so `Some(None)` never arrives and the clear is a silent no-op. `model` on the same struct is the one that uses `patch::deserialize_optional_nullable_field` | When a caller needs one of the six cleared. Each needs `#[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "deserialize_optional_nullable_field")]` — the skip is not optional, since without it every omitted field serializes as `null` and clears on arrival |
| Control-scoped writes discard the bool that denies them | `crates/workspaces/src/workspace/handlers.rs` — `create_workspace`, `update_workspace`, `override_workspace`, `add_workspace_repo`, `remove_workspace_repo`, `set_workspace_primary_repo` (nine call sites) | Needs a design decision plus a new test; see below |
| The docker preview-server readiness timeout never fires | `sandbox/docker/status_resolver.rs::inspect_server_container` | Tracked by the `test.fixme` on SERVER-02 in `tests/api/specs/sandbox-errors.spec.ts` |
| The GitHub preflight never runs on a restart | `crates/sandbox-control/src/auto_provision.rs::build_provision_request` runs only `preflight::preflight_validate_sandbox_control`; `RealPreflight`'s repo-access probe reaches only creation and task start | When `SandboxRuntime::ensure_running` no longer builds a plan on the reuse path; see below |
| Resolving a Workspace's primary repository reads every repository it places | `crates/tasks/src/task/service.rs::primary_repository_id` keeps only the first placement's `repository_id`, while `crates/workspaces/src/ports.rs::placements` fills `RepoPlacement::repository` with one `find_by_id` per placement — so a read that needs a string the `workspace_repos` row already carries pays a repository read per placement, on every task creation and every Loop firing | When a Workspace places enough repositories for the reads to show in a trace, or when something else needs placements without their repositories. A `WorkspaceResolver` method answering from `find_repos` alone is the shape; it is one more trait method for a saving nobody has measured |
| Three branch-fetch loops guess the page size instead of reading it | `crates/workspaces/src/repository/branches/providers.rs` — `fetch_github_branches`, `fetch_gitlab_branches` and `fetch_gitness_branches` each stop on `count < 100`, which asserts the host honoured the 100 that was asked for. True of those three today; not true of Forgejo, whose `MAX_RESPONSE_ITEMS` defaults to 50, and its copy of the same loop silently returned one page until #799 changed it to stop on an empty page | When a fifth provider is added, or when one of the three is next edited. One paginate helper taking a page-fetching closure collapses all four; doing it inside a Forgejo change would have rewritten three provider paths no test in this repo exercises |
| A `moat push` commit claim is process-local | `crates/adapter-github-pr/src/api_routes.rs::claim_change`, the `GithubClaimSubject::Commit` arm — `RecentlySent::note_commit` only | When a `pull_request.synchronize` is observed reaching a replica that did not serve the claim; see below |
| Every member of a shared Sandbox runs the first member's agent harness | `crates/sandbox/src/state.rs::register_task_context` — its three-field config clone leaves `agent_type` and `use_tmux` inherited from the pod, while `spawn_agent` is already called per member and `TaskContext` already holds its own `AgentHandle` | When a shared warm Sandbox needs two harnesses at once; see below |
| A Codex sub-agent's own items reach no reader | `crates/sandbox/src/agent/harness/codex/process.rs::CodexProcess::owns_thread` drops every notification from a thread this Turn did not open, which is what stops a sub-agent's `turn/completed` ending the Turn (#806). Its `item/completed` notifications go the same way, so what a sub-agent said and did renders nowhere. The parent's own `collabAgentToolCall` block still names the sub-agent and its `agentsStates` | When `agentsStates` is no longer enough to follow what a sub-agent did, or when sub-agent output is worth a stream of its own. The thread id the filter already reads is what would attribute the items, so the work is in the block and event model above it, not in this filter |
| A Task auto-close spares for its sibling Bindings is told nothing about the PR | `crates/adapter-github-pr/src/routing.rs::close_subject`, the `siblings_count > 1` arm | When a Task bound to two pull requests is observed missing the close of one; see below |
| The tar keeps its own copy of the file-set walk | `crates/sandbox/src/workspace_sync/sync.rs::append_tree` and `::append_repo`, plus `::workspace_fingerprint` — three copies of the `git ls-files -c -o --exclude-standard -z` plus whole-`.git` rule that `workspace_sync::file_set::collect` owns for the repository. `append_repo` and `collect_repo` are the pair that drifting would hurt: they would restore different trees from the same workspace | When the tar path is next touched. The tar is now only the fallback a sandbox without a repository writes, plus the rescue path, so the walk it duplicates is exercised least where it would drift most |
| The `restic` pack cache is unbounded on both sides | `crates/sandbox/src/workspace_sync/restic.rs::CACHE_DIR` and `crates/sandbox-control/src/workspace_snapshot/restic.rs::CACHE_DIR`, both `/tmp/rustic-cache`. In the sandbox `/tmp` is the `workspace-data` volume's `tmp` subPath (`pod_builder.rs:999`), so the cache spends the budget whose overflow evicts the pod; in the backend `/tmp` is an `emptyDir` with no `sizeLimit`, and it grows with every workspace ever browsed. Nothing prunes either — `prune` runs pre-stop and touches the repository, not the cache | When a soak measures either one above a few MB, or when a pod is evicted with the cache implicated. The measurement is the cheap half and belongs beside the 628 ms → 15.6 s number that makes the cache mandatory |
| A declared preview server is not routed through the preview proxy | `crates/sandbox-k8s/src/service.rs::service_port_entries` — a declared server's entry targets the port its sidecar binds, while a script port targets `SANDBOX_PREVIEW_PROXY_PORT` | When a declared server needs to bind `127.0.0.1` or enforce a `Host` allowlist, or when a starting declared server must answer with `preview_proxy::starting_page` — Traefik gets the dial failure itself and returns its own gateway error, so a declared port shows nothing until its entry targets `SANDBOX_PREVIEW_PROXY_PORT`. Pod containers share one network namespace, so the proxy already reaches every sidecar over loopback — only this function and `ingress`'s route need the new target, and the sidecar's own `containerPort` stays as it is |
| The preview proxy has no cap on concurrent connections | `crates/sandbox/src/preview_proxy.rs::serve` — each accepted connection is a detached `tokio::spawn` with no semaphore | When a preview is observed exhausting the pod's fds or memory. `UPSTREAM_HEAD_TIMEOUT` bounds the head, and the accept loop backs off once `accept` starts failing, so a stuck upstream no longer accumulates indefinitely; a bridged upgrade is still unbounded by design |
| One clone attempt has no timeout of its own | `crates/sandbox/src/init/git.rs::clone_checkout` — `run_command` awaits `git`/`gh` with no deadline, so `clone_with_retry`'s `CLONE_RETRY_BUDGET` bounds the retries between attempts but not an attempt | When a git host that black-holes rather than refuses is observed overrunning the pod's startup probe — the budget exists so init's own error reaches the termination log instead of a SIGKILL |
| The Adapter's own GitHub calls cannot see a Workspace's pinned credential | `crates/adapter-github-pr/src/auth.rs` — `client_for_outbound_reply`, `client_for_reaction`, `client_for_durable_work`, `client_for_live_status` each build an `Actor` from a login or a `TaskBinding` and never call `Actor::on_workspace`; the typed routes in `api_routes.rs` do | When a Workspace pinning a git-host credential is observed failing an event-driven reply or reaction on a repository the acting identity's own token cannot reach; see below |
| A pinned Gitness branch listing reads its base URL off the caller | `crates/workspaces/src/repository/branches/service.rs::RepositoryBranchService::list_branches`, the `RepositoryProvider::Gitness` arm — `CredentialService::provider_metadata(&ctx.user_id, "gitness")` for `base_url`, while the token beside it may have come from the Workspace pin | When a Workspace pins a Gitness credential; see below |
| A replayed `thread.create` is refused rather than recognised | `crates/server/src/t3code/backend.rs::free_thread_id` — a bootstrap frame re-sent after a reconnect names a thread that was created successfully, and the id is refused as taken. `send_turn`'s `already_sent` guard is the shape a fix would take, keyed on `NewThread::first_message`'s `message_uuid` | When a reconnecting client is observed showing "That thread already exists." on a thread it can see. The refusal predates the id-reclaim change, which only made the wording specific |
| An attached terminal stream outlives the control that opened it | `crates/t3code/src/terminal.rs::attach` / `forward_attach_stream` — control is checked once when the stream is opened (`controlled_thread`), never again while it runs, so a controller who opened a shell on a Task that is then set back to private keeps reading and writing it until they detach | When a public Task set private is observed leaving a stranger's attached shell live. The next call refuses; only the held stream is stale, and re-checking it needs a per-frame gate the attach loop does not have. Owner recovery in the meantime is `POST /cleanup` (write, owner-only) |
| An isolated Fork starts from a fresh clone, not the parent's Workspace | `crates/tasks/src/task/handlers.rs::fork_task` — the `!same_sandbox` path provisions an ordinary isolated Sandbox, so the fork's pod restores its own (empty) workspace archive at `<prefix>/<fork_task_id>/workspace.tar` rather than the parent's | When a Fork not sharing its parent's Sandbox must reproduce the parent's uncommitted working tree. See below |
| `scripts.run` answers a read-gate refusal with a defect | `crates/t3code/src/rpc/frames.rs::scripts_failure_exit` — the `ScriptsError::NotFound` arm dies with `"Conversation not found."` at `debug!`, the shape `sandbox_failure_exit` no longer has. `WsScriptsRunRpc` declares `EnvironmentAuthorizationError`, so the refusal has a type to travel as | When `scripts.run` is observed in `moatless_ui_rpc_requests_total{outcome="die"}`. It is not called today. `ScriptNotFound` beside it has no declared error to map to and needs a contract change first, so the two do not land together |
| A connection at its stream cap is told the server is broken | `crates/t3code/src/subscription.rs::StreamSet::start` — the `MAX_ACTIVE_STREAMS` refusal is a `failure_exit`, so the client renders a defect rather than a limit it could report or back off from. `EnvironmentAuthorizationError` is the one error all thirteen streaming methods declare, and it is the wrong one: it means permission, and `mapSessionRpcError` in the client turns it into a non-retryable `ConnectionBlockedError{reason:"permission"}` for `subscribeServerConfig`. A typed refusal needs a capacity error added to the contract first | When the `warn!` beside it is observed on a connection whose subscriptions are all live — that is a client holding 64 at once, which the reap no longer masks |
| A shutdown flush that overruns its budget loses the workspace silently | `crates/sandbox/src/workspace_sync/sync.rs::flush_on_shutdown` — `SHUTDOWN_FLUSH_TIMEOUT` (10s) wraps the whole of `upload()`, so the fingerprint walk and `build_tar`'s compression pass spend the budget before the PUT starts, and the cancelled future never reaches `report_workspace_sync`. Loki counts 38 `Shutdown: workspace flush timed out` against 1,367 flushes over 7 days | When a task is observed losing a turn's work after an involuntary stop. A fix either builds the tar outside the deadline or reports the timeout as a `workspace_sync_failed` event |
| A stopped Task's files are readable only out of a `restic` repository | `crates/sandbox-control/src/workspace_snapshot/restic.rs::ResticWorkspaceSnapshots` is the only reader, and `crates/server/src/compose/sandbox.rs` builds it only when `SANDBOX_WORKSPACE_REPOSITORY_SECRET` is set. A Task stored only as `workspace.tar` — a Fork whose pod only ever wrote a tar, or any sandbox that ran without the secret — answers `SnapshotError::NoSnapshot`, so the file/tree/search/asset surfaces fall back to "sandbox not running" | When a tar-only Task must serve its files without a running Sandbox. A fix reads a single file back out of the tar, which costs a fetch and a scan of the whole archive per request — the cost the repository exists to remove |
| An ongoing-mode schedule keeps the `append_system_prompt` its first firing wrote | `crates/loops/src/loops/firing/schedule_runner.rs::execute_schedule_firing` — the `RoutingMode::Ongoing` arm calls `create_scheduled_task` only when there is no open `ongoing_task_id`, and `append_system_prompt` is written once at task creation, so a Loop prompt edit and `SCHEDULE_ORIGIN_CONTEXT` both reach that task only after it closes | When an ongoing-mode schedule Loop exists. Every schedule Loop is `by_subject` today, so the arm fires for no one. A fix rewrites the column on each firing, which needs a decision on whether an edited Loop prompt should reach a task mid-conversation |
| A video has no preview URL | `crates/t3code/src/assets.rs::PREVIEWABLE` omits the video extensions `packages/shared/src/filePreview.ts` admits for a `media-file`, because `assets.rs::serve` answers from `Checkout::read`, which holds a whole file in memory and answers no range request — a video would be read wholesale to be scrubbed. The refusal is typed (`AssetPreviewTypeValidationError`), so the client renders a sentence rather than a broken frame | When the asset route can stream a range. `WorkspaceVideoPreview` (`apps/web/src/components/files/FilePreviewPanel.tsx`) cannot work until then |
| Every member of a shared Sandbox reads and writes one credentials map | `crates/sandbox/src/compose/task_context.rs::compose_task_context` hands each member's `spawn_agent` the same `Arc<RwLock<HashMap<..>>>` from `SandboxState`, and `crates/sandbox/src/agent/coordinator.rs::build_input` overwrites the whole map with `get_credentials(&self.config.task_id)` before reading it. One member's turn therefore replaces the other's credentials, and the two survive only because each member rewrites the map immediately before it reads. `crates/sandbox/src/credentials/fetch.rs::persist_agent_credentials` writes the same files from both | When two members of a shared Sandbox need different credentials — a Fork run as another identity, or two Workspaces in one pod — or when a member is observed running a turn with its sibling's token. A fix gives each `TaskContext` its own map, which the per-member `Config` already makes possible; the on-disk half needs a per-member path first |
| A Task's run-as identity is settable but not readable | `crates/api-schemas/src/task.rs` — `CreateTaskRequest::run_as_user_id` is accepted and persisted, but `TaskResponse` carries no matching field, so nothing outside the database can say which identity a Task acts as. `WorkspaceResponse::run_as_user_id` exposes the workspace-level value, so the two halves of the same precedence chain disagree on whether run-as is observable | Any change already touching `TaskResponse`'s field set, or the first time run-as has to be shown in the UI or confirmed from `moat tasks get`; until then the only way to check a Task's run-as is a `tasks` row read |
| A logout leaves the OAuth CSRF cookie standing | `crates/identity/src/auth/routes.rs::logout_handler` clears the session and CSRF cookies but not `oauth_state`, so a flow started before a logout can still satisfy the state check in `handle_oauth_callback` afterwards. The GitHub *connect* path no longer depends on that not happening — its state carries the initiating user and `github_connect::finish` discards a callback whose viewer differs — but the login paths still take a stale cookie at face value. The same face-value read reaches a second source now that the cookie carries `auth.cookie_domain`: any host under that parent can set `oauth_state`, not only read it, and under a parent covering the preview hosts that includes agent-authored code | When a second OAuth flow is added whose callback writes anything, or when the cookie is observed surviving into a later session. The fix is one more `build_clear_*` in `logout_handler`; it is not free, because a logout in one tab would then break a legitimate authorization in flight in another |
| A repository pruned only pre-stop grows without bound on a pod that is reclaimed | `crates/sandbox/src/workspace_sync/sync.rs::sync_on_request` is the only caller of `restic::forget_and_prune`, and it runs on the backend's pre-stop RPC. A node reclaim, an eviction and a SIGKILL all skip it, so a task restarted that way keeps every snapshot past `KEEP_SNAPSHOTS` (10) and every pack behind them. The same tasks that restart most are the ones that never prune | When a workspace prefix is measured holding packs for snapshots older than the last 10, or when the sync interval is shortened — the gate added with this row makes a tick cheap, so the snapshot count per hour is what rises next. A fix either prunes on a snapshot count the sandbox can read locally, or moves the prune to the backend, which sees a reclaimed task the pod cannot |
| A binding that can never resolve refreshes on every summary read | `crates/server/src/t3code/backend.rs::AppStateBoundPullRequests::bound_pull_requests` calls `refresh_incomplete_binding`, which bypasses `should_refresh`; `get_or_fetch` (`crates/loops/src/bindings/status_service.rs`) does not cache a failed fetch, so a binding whose status can never be read — no token, revoked install, deleted PR — re-attempts the GitHub call every time it is read, with no backoff. And because `bound_pull_requests` walks every cold binding on the Task, one unresolvable binding re-attempts on a summary read of *any* of that Task's pull requests, not only the broken one. Predates this path; the summary now puts it behind a per-thread sidebar chip | When a failing refresh is observed retrying on a sidebar render. A fix caches the negative result with a short TTL, or gates the refresh on `should_refresh` the way the stale-status path does |
| A UI-RPC request's wait for its connection is measured nowhere | `crates/t3code/src/rpc/socket.rs::serve_websocket` awaits `handle_json` inline in its `select!`, so requests arriving on one socket serialize behind each other, and `record_request`'s `started` is taken after dispatch — `moatless_ui_rpc_request_duration_seconds` measures the handler and structurally cannot see the wait. Over 6h on staging the client's own `RpcClient.server.refreshProviders` span reports p50 3.4s, p90 19.7s, max 48s, while the histogram puts all 103 of that tag's requests under 5s at a 0.88s mean | `moatless_ui_rpc_connection_blocked_seconds` records the wait's cause — the time the frame loop was occupied — which is as close as the loop can get: a frame is already read by the time `incoming.next()` yields it, so there is no receipt to timestamp. Attributing a wait to the request that suffered it needs the restructure: serving one connection's requests concurrently, which needs a decision first, because `handle_json` takes `&mut ProtocolSession` and the session's `config_stream` and `workspace_snapshots` assume one request at a time |
| A Sandbox held open only by a Script is reaped 15 minutes after it starts | `crates/server/src/reaper.rs::stop_idle_sandbox` — the only thing that defers a stop is `status.busy_processes`, fed solely by `CommandRegistry` (`crates/sandbox/src/task_context/models.rs`), while a Script is a PTY session `crates/t3code/src/scripts.rs::run` `exec`s through `terminal::open`. With no messages and no agent activity, `load_latest_activity_times` falls through to `get_sandbox_started_times`, and neither terminal I/O nor a published preview port refreshes that floor | When a Script run out of a draft is observed stopping under `SandboxStopReason::IdleReaper` while its port is in use. A messageless thread had no Sandbox at all before `NewThread::provision`, so the state is newly reachable. The fix is a choice between counting a live Script session as busy and adding terminal activity to the idle floor, and the two land in different bands |
| `moat tasks stop` on your own sandbox leaves no wake note | `crates/cli/src/commands/tasks.rs::run`, the `TaskAction::Stop` arm — it takes no `--yes` and records nothing, while `Restart` and `Redeploy` beside it warn first and leave a wake note through `SandboxService::RecordWakeNote`. An agent that stops its own pod is therefore stranded silently: the pod comes back on the next wake with nothing to say why it went down | When a task is found stopped from the inside with no explanation. The machinery is already there — `self_restart_note` and `record_wake_note` in the same file — so this is a call, not a design. A stop differs from a restart in one way that needs deciding: nothing restarts the pod, so the note waits for whatever wakes it next |
| `is_own_sandbox` compares task ids, so a co-member restart strands the others | `crates/cli/src/commands/tasks.rs::is_own_sandbox` — it reads `config.task_id()`, while a shared Sandbox holds several Members. Restarting a co-member's task takes down the pod this command runs in, and the guard does not fire, so no warning is printed and no wake note is left for any Member | At the first shared-sandbox restart that strands a member. A fix compares the Sandbox rather than the Task, which needs the caller to read `sandboxId` from task details — one extra request on a path that is one request today |
| Cancelling the last wake note reports success even when the file survives | `crates/sandbox/src/agent/wake_note.rs::write` — an empty map takes the `remove(path)` branch and returns `Ok(())`, while `remove` only warns on a failed delete. `CancelWakeNote` therefore answers `Ok` to a caller whose note is still on disk. Today the cost is one spare note that `take` delivers or ages out through `MAX_AGE_MS` | When a caller acts on that `Ok`. The only caller is the CLI's `cancel_wake_note`, which logs and moves on, so nothing reads it yet. A fix returns the `remove` error from `write`, which also makes `cancel` report a delete it could not perform |
| A `ui_rpc.request` span's exported duration is not its request's | `crates/t3code/src/rpc/metrics.rs::record_request` records `duration_ms`, which is truthful; the span's own start-to-end is not. `tracing-opentelemetry` restamps a span's end on every exit, and something below this surface enters a handle after the handler answered — on staging `server.refreshProviders` spans median 30.6s with `busy_ns` of 13.3ms, against a histogram that puts every one under 5s. Every other tag medians under 0.06s, so it is one path, not the surface | When the holder is named. It is not in this repo: no `in_current_span`, no `Span` in a struct field, and the only `.instrument` on this span is the one awaited to completion. Finding it needs a `Layer` recording `on_enter`/`on_close` against a live connection. `a_span_reentered_past_the_handler_stretches_but_duration_ms_does_not` pins the mechanism, and fails if the exported duration ever becomes trustworthy — at which point `duration_ms` and this entry both go |
| `--assignee null` cannot unassign a Linear issue | `crates/adapter-linear/src/reference_resolution.rs::resolve_assignee` sends `"null"` into `resolve_user_required`, which runs the login, email and display-name steps and ends at `NotFound("Linear user \`null\` not found")`, while `crates/api-schemas/src/linear.rs::LinearIssueUpdateRequest::assignee` documents `"null"` as unassign. `resolve_delegate`, one function below, intercepts the sentinel before resolving | When someone needs to unassign over the API. The fix is `resolve_delegate`'s sentinel arm, but not its shape: `resolve_assignee` returns `Option<String>` and `update_issue` writes it as `json!(a)`, so clearing needs the `serde_json::Value::Null` the delegate path already carries — a return-type change reaching both call sites |
| A reviewer Task bound by ingress hears nothing from `Audience::ParentAuthor` | `crates/loops/src/bindings/recipients.rs::resolve_recipients`, the `Audience::ParentAuthor` arm — it names producer Bindings (`origin != ADAPTER_EVENT_BINDING_ORIGIN`) and, with a parent, the Task that authored it. A Task bound by an inbound event is neither, so the review comments on the pull request it is reviewing reach it only where one replies to a comment it wrote. A top-level review comment carries `parent_external_event_id: None`, which narrows the audience to producers alone | When it is decided whether a Binding created by ingress belongs in that audience. Found on #798: with the producer Binding muted by the identity guard that PR removed, this arm was the reason the same comment also reached the reviewer Task's own Binding — one event, two independent ways to reach nobody |
| `GetAgentStatus` reports a fired wakeup as still registered | `crates/sandbox/src/agent/scheduled_job.rs` — `ScheduledJobRegistry::list` reads the registry, and a job that has fired stays in it for the length of its dispatch. Queueing the prompt made that window an enqueue RPC plus a dispatch rather than a channel send, so a status read taken inside it now shows a job that has already run | When a status read that names a fired job confuses an operator or a caller. Purely a reporting artefact: nothing schedules or cancels off `list`, and the wake itself is unaffected |
| A withheld wake's text still rides in the answer that carried it | `crates/sandbox/src/agent/dispatcher.rs::resolve_pending_prompt` withholds a self-queued row's uuid from the acknowledgment, so the row stays pending and the redispatch opens the turn it was owed. Its text cannot be withheld with it: `AgentPromptBuilder::build_pending_deliveries` joined every pending row's text with `\n` before the delivery left the backend, and the sandbox cannot tell the halves apart again. The parked `ExitPlanMode` or `AskUserQuestion` therefore reads the wake as part of the person's answer | When a delivery can carry per-row text — the fix is on the backend, not here. Until then the wake is delivered twice and lost never, which is the direction that keeps the turn recoverable |

**A Task auto-close spares hears neither notice.** `close_subject` walks the
Bindings its own trigger created, and where the Task carries other Bindings it
logs *"leaving task open"* and `continue`s — ahead of the
`build_close_inbound_message` call, so that Task gets no close notice. The
merged-PR notice does not cover it either: `merged.rs::auto_close_owns` splits
the two audiences on the structural facts a Binding carries (origin and
`loop_id`), which is what keeps `github.close` and `github.pr_merged`
independent of the order they execute in, and the sibling rule is not one of
those facts — it is a per-Task database read `close_subject` makes at execution
time. Closing the gap belongs in `close_subject`, which already knows it is
sparing the Task: deliver the notice before the `continue`, and
`auto_close_owns` keeps working unchanged.

**The Adapter's own GitHub calls and a pinned credential.** A Workspace that
pins a git-host credential has it resolved by `GitHostTokens`, which reads the
Workspace off the `Actor` — so a caller reaches the pinned account only by
building its Actor with `Actor::on_workspace`. `api_routes.rs` does, through
`task_workspace`, which is why `moat gh pr create` / `pr comment` / `pr reply`
present the pinned account. The four `auth.rs` constructors above do not: they
run on webhook and task-event paths whose Actor comes from a GitHub login or a
`TaskBinding`, and neither names a Workspace.

Closing it needs a task-to-Workspace read those paths can make, and there is
none that fits: `TaskReader::describe` takes a `ScopeContext`, which background
work does not have and must not fabricate — `ScopeContext::system` is fenced off
in `clippy.toml`. So this is a new port on a published seam plus the
authorization question of what may read a Task's Workspace outside a request,
which is why it is not folded into the change that found it.

**A pinned Gitness listing and its base URL.** A branch listing resolves its
token through `GitHostTokens`, which honours a Workspace pin. Gitness needs a
second value the other hosts do not — the deployment's `base_url` — and
`list_branches` reads it from `CredentialService::provider_metadata`, keyed on
`ctx.user_id`. So the
token can come from the Workspace and the URL from the caller, and a caller with
no Gitness row of their own is told *"Gitness base URL not configured"* even
though the pin holds a working credential.

Nothing pins Gitness today, which is why this is recorded rather than fixed:
closing it is a decision about where a Gitness deployment's address lives.
`workspace_credentials` stores one opaque value per `(workspace, provider)` and
has no metadata column, so either the pin grows one, or the base URL is declared
on the Repository beside the remote URL it already carries. Both are wider than
a branch listing, and picking one without a Workspace that pins Gitness would be
guessing at the shape.

**Workspace control-filter writes.** `WorkspaceDao`'s `update`, `add_repo`,
`remove_repo` and `set_primary` all return `bool`, and the trait documents it:
*"Returns `false` when nothing changed, row is missing, or caller cannot control
the row."* Nine call sites across six handlers discard it, then re-fetch under the
**read** filter and return 200 — so a caller who may read a workspace but not
control it gets a success response for a write that did nothing.
`delete_workspace` in the same file already does it right
(`let deleted = …; if !deleted { NotFound }`) and is the precedent to follow.

`override_workspace` is the sharpest case: its own comment states that
*"the DAO `control_filter` ensures only the owner/admin can do this"*, and it
then discards the only signal that `control_filter` matched nothing. The code
documents a guarantee it does not check.

The fix does not transfer from the repository equivalent, which is why it is
deferred rather than copied. `PgRepositoryDao::update` always binds `updated_at`,
so `rows_affected() > 0` holds for any authorized caller and `false` means
control-miss alone. `PgWorkspaceDao::update` returns `Ok(false)` for an empty
patch *before* reaching the control filter, so a naive `if !applied { NotFound }`
would 404 an authorized no-op `PATCH` — a new bug shipped as a security fix. Pick
one of: short-circuit the empty patch in the handler, give the DAO a return type
that separates "nothing to do" from "you may not", or always push `updated_at`.
Whichever is chosen, the empty-patch path needs a test; nothing covers it today.

**Preview-server readiness.** A server whose start command never binds its port
should reach `failed` after `startupTimeoutSeconds`. Under docker it stays
non-terminal. The gap is one shape — an absent `StartedAt` is treated as "keep
waiting" — across four paths: `running` with the port closed and `StartedAt`
absent or unparseable falls through to `Starting` with no `else`; `created` and
`restarting` return `Starting` with no timeout check at all; the catch-all and
the inspect-404 arm both return `Stopped`. None is terminal to the poll.

Fixing the match arms alone is not enough and cannot be written as such: the
function receives only the container name, port and timeout, so on the arms where
the container never started there is no timestamp to measure elapsed time
against. The fix threads a "start requested at" instant through the signature.
The doc comment on that function is accurate — it scopes its guarantee to
`running` — so this is a design gap, not a stale comment; update it when the
behaviour widens.

**The GitHub half of the preflight on the provision path.** A task whose GitHub
token has been revoked since it was created still gets a pod on restart, and
fails in-pod at clone time rather than as one `sandbox.error` before any
infrastructure exists — the same shape the control check now closes.

The gate cannot simply widen to the full `PreflightPort`, which is why this is
recorded rather than done. `K8sSandboxAdapter::ensure_running` and
`DockerSandboxAdapter::ensure_running` call `ProvisionDeps::build_request`
unconditionally, before their ready-container reuse paths, and every message
wake of a running task goes through it. A `GET /repos/{owner}/{repo}` there
would put message delivery behind GitHub's availability and bill a call per
message, and the rejection class is permanent — a GitHub 5xx would fail the
wake outright. The control check is one indexed read and carries none of that.

Widen it when `ensure_running` can tell "reuse a live sandbox" from "create or
start one" without building the plan first; the probe then belongs on the
second branch only.

**The push claim across replicas.** A `pull_request.synchronize` names only the
new head commit, and the claim is matched against it by prefix in
`RecentlySent` — process-local, 120-second TTL. `outbound_claims` cannot hold it
as it stands: it looks up by equality on an id, and a commit claim is a prefix
match. The exposure is narrow, because the claim now carries the *full* sha (the
CLI resolves it with `git rev-parse` before pushing) and the notification arrives
within seconds of the push. Give it a durable home when a synchronize is actually
observed reaching a replica that did not serve the claim — an equality index on
the full sha would do it, and the prefix match only exists for the abbreviated
shas the old transcript scan had to work from.

**Per-member agent harness.** The pod already builds one harness per joined
Task: `register_task_context` calls `spawn_agent` for each member and the
resulting `TaskContext` holds its own `AgentHandle`, and the image carries both
CLIs. What it does not do is vary the harness. The function clones the pod
config and overrides three fields — `task_id`, `workspace_dir`, `repo_dir` —
so `Config::agent_type` and `Config::use_tmux` stay whatever the pod was
provisioned with, which is the first member's selection. A second member that
asked for a different agent silently runs the first one's.

This has no effect while a Sandbox has one member, which is every Sandbox that
does not use `tasks.sandbox_id`. It becomes wrong the first time two members of
a shared warm Sandbox want different harnesses.

The shape of the fix is known and small: a field on `JoinTaskRequest`
(`crates/sandbox-proto/proto/sandbox/v1/sandbox.proto`, which today carries only
`task_id` and `checkout_mode`), two lines in `register_task_context` to override
the cloned config, and the value threaded from the joining Task through
`SandboxJoiner::join_task`. It is deliberately not part of the Task-controls-
Sandbox change, because it moves in the opposite direction: that change made the
harness pod shape (`sandboxes.agent_type`, `sandboxes.use_tmux`), and this one
makes it per member again. Do it as its own change, with the pod-level columns
kept as the default a member inherits when it asks for nothing.

**An isolated Fork's Workspace.** A Fork carries a copy of the parent's
conversation (`message::fork::copy_conversation`) and, on its first Turn, resumes
the parent's Session. The session state reaches an isolated Fork's pod —
`DownloadTranscript` serves the source Task's archive to a Fork that has none of
its own — but the working tree does not: the Fork provisions an ordinary Sandbox,
whose init restores `<prefix>/<fork_task_id>/workspace.tar`, its own still-empty
archive, so the pod holds a fresh clone and none of the parent's uncommitted
edits. A `sameSandbox` Fork has no such gap: it shares the parent's checkout.

Closing it is a seed step before the Fork's Sandbox provisions: S3 `copy_object`
of the parent's `workspace.tar` onto the Fork's key (`sandbox_key` in
`crates/server/src/sandbox/grpc_server.rs`, `task_id`-scoped). The copy needs
the S3 client, which lives in the backend band, not `moatless-tasks` where
`fork_task` runs — so the seed belongs on the provisioning path, keyed off the
`forked_from` origin, not in the create handler. A `copy_object` also makes the
Fork's own archive exist from birth, which the transcript path deliberately
avoids: its fallback is keyed on the Fork having no archive yet, so that is how a
restart after the Fork's first Turn stays on the Fork's transcript.

Neither archive is flushed at the cut. `transcript_sync` uploads every 60s and on
shutdown, so a Fork taken moments after a Turn ends can read an archive that stops
short of its own anchor. That degrades to a fresh Session rather than a wrong one:
`write_forked_transcript` refuses a cut whose anchor names no line. Forcing it
would be a flush of the parent before the Fork provisions — the
`SyncWorkspaceRequest.include_transcripts` toggle is the hook — and is worth doing
when a Fork is observed losing context on a parent whose pod is still running.

## Observability

| Deferred | Anchor | Entry condition |
|---|---|---|
| A blocked storage reclaim reaches no panel and no rule | `crates/sandbox-runtime/src/metrics.rs::STORAGE_RECLAIM_TOTAL` — the dashboards under `k8s/helm/moatless-vibe/dashboards/` query it nowhere | When someone next edits `k8s/helm/moatless-vibe/templates/prometheusrule.yaml`. `blocked` means the archive could not be proven current and the rescue did not land, so it is the one reclaim outcome that costs a workspace |
| The live Grafana dashboard is not provisioned from this chart | `k8s/helm/moatless-vibe/dashboards/moatless-vibe-live.json` against the `Moatless Vibe Live` dashboard in the Soap Grafana workspace, whose `meta.provisioned` is `false`. `k8s/helm/moatless-vibe/values.yaml::monitoring.grafana.enabled` defaults to `false`, so nothing applies this file. Anyone who edits it must apply the same change through the Grafana API, or the two diverge. The live copy is missing the `Sandbox Storage (EBS volumes)` row and its 9 panels today | When `monitoring.grafana.enabled` is turned on, or when the drift next costs a reader a wrong answer |
| `node_reclaimed` reaches no counter, so no rule can fire on it | `crates/sandbox-k8s/src/pod_watcher.rs::process_watcher_event` — `sandbox_container_terminations_total` is incremented inside the `transition.is_error_event` branch, and `crates/sandbox-k8s/src/pod_observation_policy.rs` maps only `PodEvicted` and `PodFailed` to `SandboxStatus::Error`. Every other way a pod dies takes the `else` arm and is logged only. Over the last 7 days that hid 1057 `node_reclaimed` events across 248 tasks — a spot instance taken back is the fleet's largest single cause of a lost sandbox — against 22 `pod_evicted` the counter did see | When the `Sandbox Lifecycle Events` row on `moatless-vibe-live.json` is not enough, which is the first time anyone wants an alert rather than a panel. Widening the counter is the fix, not widening `is_error_event`: that flag also forces every server container to `Failed`, and a reclaimed node is followed by a replacement pod |
| No rule fires on a workspace sync failure or a mirror restore | the same `prometheusrule.yaml`, which holds one rule, `LinearAdapterAuthFailing`. The `Workspace Sync (restic / mirror)` row on `moatless-vibe-live.json` plots both, so only a reader who opens the dashboard sees them | The same pass. A mirror restore needs care as an alert: `crates/sandbox/src/workspace_sync/restic.rs::restore_on_start` returns `Absent` on three paths, and one of them has already recorded a restic download |

## Test and CI structure

| Deferred | Anchor | Entry condition |
|---|---|---|
| No CI job type-checks an E2E-only change | `.github/workflows/ci.yml` — `type:check:e2e` runs only inside `Build frontend`, gated on the `frontend` / `docker-frontend` path filters (`apps/frontend/**`, `packages/**`). `tests/**` is in no filter, so a PR touching only `tests/` skips both `Build frontend` and `Lint frontend` | When an E2E-only change lands broken on `main`, or alongside the next paths-filter edit. Until then `bun run type:check:e2e` before merge is the substitute |
| The E2E client hand-writes routes it could generate | `tests/shared/moat-api.ts` — 65 raw `(method, path)` literals | When route drift next costs a run, or alongside any broad route change |
| `E2E_GITHUB_PAT` is unset on the repository | `tests/shared/profile.ts` gates `githubRepositories` on it | A repository-settings change; un-skips 7 tests |
| No `cargo fmt --all --check` gate | `ci.yml`; a standing diff in `crates/adapter-github-pr/src/api.rs` | Sweep and gate together, so the gate starts green. The sweep needs a `rustfmt.toml` pinning `style_edition = "2024"` first: the workspace declares `edition = "2021"`, so `cargo fmt` formats to the 2021 style while the tree is written in the 2024 one, and a sweep without the pin rewrites the tree the wrong way. See `rust-dev-guidelines`, "Format single files, never the crate" |
| Nothing alerts when `main` goes red | `ci.yml` | Pairs with a nightly build for the image tier |
| Nothing holds `Dockerfile.e2e`'s Playwright pin against the lockfile | `docker/Dockerfile.e2e::ARG PLAYWRIGHT_VERSION` versus the `@playwright/test` resolution in `bun.lock`. `docker/e2e-entrypoint.sh` runs `bun install --frozen-lockfile` and never `playwright install`, so the baked browsers and the installed client must be equal or `tests/ui` dies at `browserType.launch`. The file's own header states that rule and it drifted anyway. The caret range means an ordinary lockfile refresh onto a new minor re-breaks it silently, and the only job that builds the image is `e2e.yml`, which is `workflow_dispatch`-only | A sibling step in `ci.yml`'s `toolchain-drift` job, shaped like the `rust-toolchain.toml` ↔ `ARG RUST_VERSION` comparison beside it: `sed -n 's/.*\["@playwright\/test@\([0-9][^"]*\)".*/\1/p' bun.lock \| head -1` against `sed -n 's/^ARG PLAYWRIGHT_VERSION=\(.*\)$/\1/p' docker/Dockerfile.e2e`. Written and verified both ways on #760 — green on the fixed tree, red on the 1.56.1 that PR replaced — but not landed: the agent GitHub App has no `workflows` permission, so it cannot push `.github/workflows/`. Needs a human with that scope |
| `a_detached_operation_gets_a_root_span_linked_back_to_the_caller` fails under test parallelism | `crates/sandbox-control/src/lifecycle/orchestrator.rs` — the test finds no `sandbox.lifecycle` span about two runs in three under `cargo test -p moatless-sandbox-control`; alone or with `--test-threads=1` it passes every time. The recorder is installed with the thread-local `tracing::subscriber::set_default`, and `Lifecycle::start` detaches the span-opening work | When it fails in CI, or when anyone next works on `Lifecycle::detached_span`. It has been failing since before the workspace-repository work and is invisible today because the crate's tests run interleaved with enough others to usually win the race |
| A sandbox readiness test intermittently hangs to the CI timeout | `crates/sandbox/src/init/replacement_test/readiness.rs::sustained_ack_failure_never_re_runs_and_aborts_on_shutdown` | When it fires a second time on `main`, or when anyone next works in `crates/sandbox/src/init` |
| Nothing holds a removed crate edge removed | `ci.yml`; `crates/ports/Cargo.toml` can regain `sandbox-proto` and `crates/core/Cargo.toml` can regain `moatless-api-schemas` with no gate objecting | A `cargo metadata` check over a forbidden-edge list, sized so one line per edge is the whole cost of adding one |
| The four tmux `agent_transport_e2e` tests hang instead of failing | `crates/sandbox/tests/agent_transport_e2e.rs` — the `*_tmux` tests; the first one times out at `wait for session start` and the run then never exits, so a 1500s deadline killed it | When anyone next works on the tmux transport, or when the CLI-upgrade loop needs the TUI transport covered — it runs with `--skip tmux` until then, so a Claude CLI bump is verified over the subprocess transport only |
| The agentops bot's Codex sign-in is dead | `$CODEX_HOME/auth.json` in a sandbox run by `soap-agentops[bot]` — access token expired 2026-04-09, and the server reports the refresh token already spent, so it cannot be refreshed headlessly | When a Codex CLI bump needs verifying. Until it is re-authed the `agent-cli-upgrade` loop cannot exercise the Codex transport, so it opens no PR for a Codex bump at all — `LOOP.md` blocks on a dead credential for the CLI being bumped. A Claude-only bump still ships, with the Codex gap named in its PR body |
| Committed OpenAPI spec and TS client lag the backend | `openapi-specs.json` and `packages/api-client/src/**` lack the `fork_task` operation (`POST /api/v1/tasks/{task_id}/fork`) the tasks routes register | Run `task generate` alongside the next change touching the API surface, or on its own when the frontend first consumes the fork endpoint. Harmless until then: `build-rust` regenerates the spec fresh on every push (the `Generate OpenAPI spec` step in `ci.yml`), so `main` stays green while nothing references the missing types |
| The committed TS client was generated by an orval the repo no longer resolves | `packages/api-client/src/**` carry a `Generated by orval v8.2.0` banner; `packages/api-client/package.json` declares `orval: ^8.29.0`, and 8.29 drops the version from the banner and re-indents every multi-line JSDoc continuation. So `bun run generate:client` rewrites all ~380 files, and any PR that adds one API field either hand-writes the generated file or buries its diff | When someone regenerates on its own commit, so the churn is reviewable as churn. Harmless until then — `build-rust` regenerates the client fresh on every push, and nothing gates on the committed copy but frontend type-check |
| The restored Rust cache costs ~90s in every Rust job | `ci.yml` — `rust-clippy` and `rust-test` restore one 2035 MB entry that unpacks to 7.0 GB of `target/debug`, of which `deps` is 6.9 GB; it is a quarter of each job's wall time | Feature variants are the cost: `.github/scripts/measure-rust-cache.sh` measured 1153 rlibs across 450 crate names, and eight 103 MB copies of `libaws_sdk_s3` alone are 826 MB. The clippy job compiles 8 distinct feature sets into one target dir. Entry condition: when the restore is worth a `CARGO_TARGET_DIR` split per feature set. `aws-sdk-s3` stays in the workspace-wide graph: `moatless-core` names `moatless-s3` for the file operations it owns, so every member above core carries it |
| The `architecture` job has no Rust cache | `ci.yml` — the `architecture` job runs `cargo test -p moatless-arch-rules --locked` with no `Swatinem/rust-cache` step, so it cold-compiles `syn`, `toml` and `cargo_metadata` every run; 215s of a 243s job against ~40s of rule work | When the job reaches its `timeout-minutes: 10`, or when it becomes the run's tail. It is off the critical path today — `Rust tests` is |

**Route literals.** The suite already imports generated *types* from
`@moatless/api-client` (`tests/shared/types.ts`, `tests/shared/wait-helpers.ts`),
and `bun run type:check:e2e` now runs in CI, so type drift fails at the commit.
Route and method drift is caught by nothing until a run. That asymmetry is why a
single upstream change fans out into a cluster of red tests rather than one
compile error — `PUT` vs `PATCH` and `/reset` vs `/reset-git` both reached CI
this way. Driving the routes off the generated client is the change; it is
sizeable and belongs on its own.

## Dead or misleading

| Deferred | Anchor | Entry condition |
|---|---|---|
| No CI job executes the db-backed integration tests | The db-tests leg is `cargo nextest run --no-run` and no job provides a database, so `crates/server/tests/integration_*/` compiles and is never run. A change that renames a table, a payload field, a route or an event identity therefore leaves these tests asserting the old one and CI still green; the drift is only visible to whoever next points a Postgres at them. Last count: 51, measured against Postgres 16.2 on 2026-09-01 — see below | Standing up a Postgres service for the db-tests leg, which is the change that stops this recurring |
| The committed TypeScript API client predates its own generator | `packages/api-client/src/sandbox/**` carries `Generated by orval v8.2.0`, while `packages/api-client/package.json` pins `^8.29.0`. Re-running `task generate:client:ts` rewrites all 411 files (~15k lines) and adds a `src/sandbox/index.ts` the tree does not have, so any change that touches the client either hand-edits generated files or drowns its own diff. Nothing catches it: `check:spec` diffs `openapi-specs.json` only, and the client is not regenerated in CI | A change whose own client diff is already wide, so the regen churn lands on its own commit and is reviewed as the version bump it is |
| The `AdapterNewThread` outbound capability is dead in production, live in tests | `bindings/outbound.rs::direct_send_to_rule` (sole production caller) reaching `capabilities.rs::{AdapterNewThread::send_message, binding_destination_from_new_thread}`, `registry/mod.rs::{Registry::outbound_for_new_thread_by_name, OutboundOperation::NewThread}`, both impl'd by `providers/{slack,telegram}/outbound.rs` | Is subscription-based new-thread outbound being finished or abandoned? See below |
| The sandbox images are documented as two products | `docs/docker-builds.md`, `docs/sandbox-architecture.md`, `docker-compose/README.md`, the `docker-install` and `docker-testing` skills, and the commented alternative in both `.env.example` files | Any pass over the sandbox image docs |
| Two crates mint the same WebSocket ticket | `identity/src/auth/jwt.rs::generate_ws_ticket_jwt` and `t3code/src/auth.rs::generate_ws_ticket_jwt` — same `moatless-vibe-backend/ws-ticket` issuer, same 60s TTL, same claim fields under two names (`SessionClaims`, `UiRpcSessionClaims`); they differ only in which identity type they take | Any change to the ticket's issuer, TTL or claim shape, because that change has to land in both and nothing makes the second copy visible |
| `DownloadUpload` is served for nobody the repo still builds | `sandbox.proto::BackendSandboxService.DownloadUpload` and `grpc_server.rs::download_upload` — the single-message upload RPC. No sandbox built from this tree calls it; it exists because a running pod keeps the image it was provisioned with, so pods older than `DownloadUploadChunked` still do. Removing it takes the RPC, the handler, and `DownloadUploadResponse` (the sandbox's `BackendClient::download_upload` returns that message but reassembles a chunk stream to build it) | When no sandbox pod predating `DownloadUploadChunked` can still be running — the horizon is the longest-lived task's sandbox, so a deployment-wide redeploy settles it immediately |
| Two of the task builder's four `go get` pins clear no finding | `docker/Dockerfile.sandbox-base`, the `task-builder` stage's `go get`: `go.opentelemetry.io/otel@v1.45.0` and `golang.org/x/net@v0.58.0`. The otel finding they were added for (OP-760, GO-2026-5158 against `otel@v1.41.0`) is kubectl v1.36.2's vendored copy, not go-task's — `ARG KUBECTL_VERSION=v1.37.0` is what cleared it, and `TASK_VERSION=v3.52.0` already resolves otel v1.44.0. `x/net` has no OP-760 finding at all, and the pin sat *below* go-task's own v0.56.0 before it was raised. Both are harmless, but a reader takes them as evidence of a finding they cannot find | When an otel or `x/net` finding is next attributed to this block, or when the block is next edited. A scanner reports every Go binary in the image, so match the reported version against `go list -m` in the builder stage before adding a pin here — the binary that carries it is often `kubectl`, not `task` |
| A sandbox reports no aggregate resource usage | `moatless_api_schemas::sandbox::ResourceMetrics` and `SandboxStatusFullResponse::resources`. All four status resolvers (`sandbox-k8s/src/status_resolver.rs:144,254`, `sandbox-docker/src/status_resolver.rs:133,180`) hardcode `resources: None` and nothing downstream fills it, so the type and its wire key are dead on every deployment. The doc — "Resource metrics from sandbox GET /status" — and the `event_loop_latency_ms` field name it still carries say where it came from: the Node sandbox agent's own status endpoint, which nothing reads. Real per-container CPU and memory do exist, on `container_diagnostics[]`, from `fetch_container_resource_usage` | Delete the type and the key, or fill it from the metrics read `fetch_container_diagnostics` already does. Whichever comes first — a consumer that needs a per-sandbox total (the T3 panel sums `container_diagnostics[]` instead), or any pass over the status resolvers. Uptime has no source anywhere and would need a pod start time to fill |
| The secret store holds values that are not secrets | The `secrets` table, `crates/identity/src/secret/`, `SecretService`, `/api/v1/secrets`, `moat secrets`, `apps/frontend/src/features/secrets/` — the store is scoped, keyed and delivered into sandbox runtimes, and a Secret is one flavor of what it holds. The other flavor is a **Variable**: readable, listable, non-sensitive configuration such as the Claude Code toggles in `crates/sandbox/src/init/claude_code_settings.rs`. `Variable` is the agreed umbrella name; `Secret` stays the sensitive flavor | When a wire-contract release is already breaking `/api/v1/secrets`, or when the plain flavor grows enough surface that "secret" misleads whoever reads it next. The rename is mechanical but wide — ~204 files, a table rename, and a breaking change to both the public API and the `moat` CLI — so it is not worth a pass of its own |

**The db-tests count.** The tier is where every SQL statement in the codebase is
exercised, so what it does not run, nothing runs. The 51 break down as 4 in
`integration_adapters`, 11 in `integration_data`, 36 in `integration_surface`;
415 db-tests pass. None of the failures are subtle — a `users` row that predates
the `users_human_email_required` constraint, an adapter kind list missing
`github.pr_merged`, a projection emitting `scriptsEditable` against a contract
that does not declare it, seed rows colliding on `sandboxes_pkey`. Each is a test
asserting a shape the code moved past.

Two things make this cheaper to schedule than the count suggests: the failures
are concentrated, with three files holding 47 of the 51, and reproducing the list
needs only a throwaway server —

```
DATABASE_URL=postgres://…  cargo test -p moatless-backend --features db-tests
```

`task_listing_position_test::no_mutation_holds_the_counter_while_it_waits_for_another_lock`
is timing-dependent and flaked once across those runs; it belongs on the flaky
list rather than in a fixture pass.

**Subscription-based new-thread outbound.** `direct_send_to_rule` — documented as
"the subscription-based replacement for direct sends to Slack/Telegram adapter
configs" — has no caller anywhere, production or test. It is the *only* production
caller of the `AdapterNewThread` capability: deleting it orphans
`AdapterNewThread::send_message`, `binding_destination_from_new_thread`,
`Registry::outbound_for_new_thread_by_name`, and the `OutboundOperation::NewThread`
variant — all of which Slack and Telegram implement and unit tests still exercise
(two are named `binding_destination_from_new_thread_preserves_direct_send_shape`).
The capability is therefore dead in production and live in tests. The condition is
not size but a product fact neither the AppState sweep nor a cleanup PR can settle:
is subscription-based new-thread outbound a feature being *finished* or one
*abandoned*? If finished, `direct_send_to_rule` gets wired to a route; if
abandoned, the function and the whole capability — the trait method, both provider
impls, the registry wiring (`register_new_thread` + the `new_threads` map), the
`NewThread` variant, and their tests — come out together. Whoever knows which
answers it in a minute; without this note the next reader re-derives the entire
call graph, which is what #403's follow-up did before writing this.

**Sandbox image docs.** These describe a slim JavaScript image and an alternative
full-polyglot image as two things you can choose between. CI builds one image
from `docker/Dockerfile.sandbox` and pushes it under two repository names —
`moatless-sandbox` and the back-compat `moatless-sandbox-js` alias — while
`Dockerfile.sandbox-full` is local-only and published nowhere. The "alternative"
the docs offer is an image nobody builds.

## API surface

Signatures broader than their callers need. Not a defect — the code is correct —
so each waits for a change already touching the same signature rather than a pass
of its own.

| Deferred | Anchor | Entry condition |
|---|---|---|
| Access-administration compatibility aliases | `crates/api-schemas/src/provider_settings.rs` deprecated Rust aliases and `packages/api-client/src/sandbox/model/provider-settings-compatibility.ts` | Remove the old DTO and `sandbox/provider-settings/provider-settings` TypeScript exports in the next major wire-contract release |
| `TaskEventSubscriber` has one production implementation | `crates/loops/src/adapters/events.rs::TaskEventSubscriber` — Linear is the only Adapter that reacts to a task's own events; the seam's own tests supply the second and third | When a second Adapter earns live status — Slack and Telegram react to no agent event today. Not a candidate for collapsing: deleting the seam puts eleven provider dispatches back into `event_handler/`, which is what it exists to keep out |
| `PluginDao::clear_activation` takes an owned skill name | `crates/workspaces/src/plugin/dao.rs::PluginDao::clear_activation` — `skill_name: Option<String>` | Nothing requires the owned form; narrow it to `Option<&str>` when this signature is being changed for another reason, rather than touching a request path for that alone |
| The UI-RPC subscription-open arms hold the frame loop | `crates/t3code/src/rpc/dispatch.rs::ProtocolSession::handle_request` — `terminal.attach`, `sandbox.subscribeStatus`, `subscribeServerStatus`, `orchestration.subscribeThread`, `orchestration.subscribeShell`, `subscribeServerConfig`, `subscribeTerminalMetadata`. Each awaits a seed read *before* it registers its stream, so unlike the reads `detach` moved off, the await is inside the frame loop. Detaching one means sharing `StreamSet` across tasks, and its `start_produced` spawns while holding the set — a lock there would be held across an await | When `moatless_ui_rpc_connection_blocked_seconds` p95 is still above ~100ms with the detached reads gone. Until then the blocking left on the loop is the seed reads, which are single round trips, not the multi-second handlers that motivated the split |
| `UserInfo` carries no `status` | `crates/api-schemas/src/auth.rs::UserInfo` — `PATCH /api/v1/users/{login}` returns it, so `moat user deactivate --json` prints the value it sent rather than the value the server stored. A scripted caller has nothing to assert on and has to re-read `moat user list` | When the next change already touches `UserInfo`. Seven construction sites build it, including the login and session-status payloads, so adding a field to confirm one CLI read-back is not worth touching those paths on its own |
| The admin users sidebar neither shows nor sets `status` | `apps/frontend/src/features/admin/components/admin-users-sidebar.tsx` — it reads `GET /api/v1/users` without `includeDisabled`, so an admin cannot see a disabled account, let alone re-activate one. Both directions are CLI-only | When an admin needs to undo a disable from the UI. #797 disabled accounts for an agent that picks a ticket assignee, and that caller never opens the UI |
| A disabled account's API key survives in the auth cache for up to 30s | `crates/identity/src/auth/api_key.rs::validate_key_cached` — a cache hit returns the stored `VerifyPayload` without re-reading `users`, so the `status` check in `validate_key` is skipped until the entry ages out. `AUTH_CACHE_TTL_SECS` (default 30) bounds it, and `middleware.rs` takes this path for every `Bearer mvk_` request, not only `verify_handler` | When a disable has to take effect inside that window. `revoke_api_key` already invalidates by key hash; a disable would have to enumerate the user's keys to do the same, which is a DAO method that does not exist yet |
| The last admin can be disabled, and then nobody can re-enable anybody | `crates/identity/src/auth/accounts.rs::update_user` — the handler refuses a self-disable, and stops there. A second admin disabling the last remaining one leaves `PATCH /api/v1/users/{login}` unreachable, and recovery is a manual `UPDATE users`. `ensure_local_admin` does not rescue it: `password/bootstrap.rs` passes `None` for `status`, so a restart restores the admin role on a row that stays `disabled` | When a deployment has more than the one or two admins it has today. A fix counts the remaining active admins on every status change, which puts a query on a path that runs for a name edit too |
| A WebSocket ticket outlives a disable by up to 60s | `crates/identity/src/auth/middleware.rs` — the ticket arm verifies a JWT and reads no user row, so a disabled account holding a fresh ticket completes one upgrade. `WS_TICKET_TTL_SECS` bounds it at 60 seconds, and every other credential is refused on the next request | When a disable has to take effect inside that window. A fix reads the user row in the ticket arm, which puts a database round trip on every upgrade |
