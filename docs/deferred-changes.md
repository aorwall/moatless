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
| `SandboxWakeService`'s closed-task guard is untested | `crates/sandbox-control/src/notify.rs::SandboxWakeService::wake` — the "a closed task does not wake" branch; only the seam (`moatless_backend_core::sandbox::wake_for_message`) is covered, not the wake behaviour | When the rich `Lifecycle` is behind a trait and `SandboxWakeService` is constructible from fakes — the guard now reads `Lifecycle::intent` and the start calls `Lifecycle::start`, so one trait covers both |
| `TaskReaper`'s two above-band concrete fields | `task/reaper.rs::TaskReaper::new` — `k8s: K8sClient`, `sandbox: Arc<Lifecycle>` | Step 10d — a precondition of the crate compiling, not a cleanup |
| Credential broadcast fan-out reaches `tasks` | `credentials::CredentialService::store` auto-invokes `broadcast_to_scope` | Step 10c, and it must land with the fan-out test that does not exist |
| `sync_config`'s body spans two bands | `repository/handlers.rs::sync_repo_config` runs a loop sync and a workspace sync | Step 10g, as a server-band composition |
| The error-type knot | `error.rs` split, 7c's registry inversion, 8b's remainder, 8d's error half | One knot, one entry condition, recorded in the plan |
| `SetupRequirementCheck` is a single-implementation trait | `setup/evaluator.rs::SetupRequirementCheck` — `checks()` returns `vec![Box::new(GithubTokenRequirement)]`; recorded as 8h | When it earns a second implementation, or is collapsed into `evaluate_setup` |
| `moatless-api-schemas` is one crate for every domain | `crates/api-schemas/src/` — 15 crates depend on it, and each consumer already has a `<domain>/schemas.rs` that is one `pub use moatless_api_schemas::<domain>::*;` | When seven or eight more crates is an accepted cost. It is the single largest rebuild-cost edge in the workspace and the split itself is mechanical; the crate count is the decision |

**On `AgentErrorSink` specifically.** It is a consumer-owned port with exactly one
production implementation, created so the reaper stops naming `AppState` and comes
off `scripts/appstate-composition-root-baseline.txt`. It clears the repository's
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
`sandbox-control` outright. It is declared in `moatless-backend-core` beside the
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
| `CredentialError` names the store | `crates/workspaces/src/credentials/error.rs::CredentialError` — `Database(#[from] sqlx::Error)` | When a third crate outside `workspaces` names it. `backend` and `tasks` do today, one short of the bar |

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
| `User` and `TeamMember` are not in `moatless-types` | `crates/identity/src/auth/users.rs::User`, `crates/identity/src/team/models.rs::TeamMember` — both key on `uuid::Uuid` | `uuid` is now on the list, so the block is gone. Move each one when a third crate outside `identity` names it |

## Table ownership

`scripts/check-table-ownership.sh` fails on a production SQL statement naming a
table another crate owns, against the map stated in the script.
`scripts/table-ownership-baseline.txt` holds the statements that predate it, each
with its reason, and only shrinks. Two decisions sit outside that file.

| Deferred | Anchor | Entry condition |
|---|---|---|
| `t3code` reads fifteen tables it does not own | `crates/t3code/src/lib.rs` — 43 statements across `tasks`, `task_messages`, `task_turns`, `task_activities`, `task_follows`, `task_bindings`, `task_message_usage`, `task_message_contexts`, `workspaces`, `workspace_repos`, `users`, and the four listing/conversation change-feed tables. It is now declared a read-only projection in `crates/t3code/AGENTS.md`, which is what the table-ownership exclusion rests on | When one of these reads has to become a write, or the T3 contract changes shape. Either breaks the declaration and forces the reads through the ports |
| `adapter-telegram/pairing.rs` is mostly unreachable | `crates/adapter-telegram/src/pairing.rs` — `find_by_code`, `list_pending`, `list_pending_for_user`, `approve_request`, `prune_expired` are `pub` with no caller anywhere in the workspace; only `upsert_pairing_request` is reached (from `ingress.rs`) | When the Telegram pairing flow is next worked on. Deleting them now would be a behaviour question dressed as a cleanup — nothing says whether the flow is unfinished or abandoned |

## Seam gates

`scripts/check-crate-seams.sh` invariant 1 keeps a store private to its crate by
matching references to a `*Dao` path. A concrete store that is not named `*Dao`
is outside what it can see, and one such store crosses crate boundaries today.

| Deferred | Anchor | Entry condition |
|---|---|---|
| A concrete Postgres store is held by five peer crates | `crates/loops/src/adapters/delivery/store.rs::AdapterDeliveryStore` — `pub` for `new` and `admit`, the other nine methods `pub(crate)`; the gate does not see it because the name carries no `Dao` suffix | When a second admission implementation exists. A port over `admit` would have one implementation, and the only test that could use a fake — `crates/loops/src/adapters/ingress.rs::store_with_no_database` — deliberately uses a lazy pool pointed at a dead address instead, so the port would not clear the repository's single-implementation bar |

## Correctness

Defects found by reading, each real, none urgent enough to have been fixed in the
change that found it.

| Deferred | Anchor | Entry condition |
|---|---|---|
| Control-scoped writes discard the bool that denies them | `workspace/handlers.rs` — `create_workspace`, `update_workspace`, `override_workspace`, `add_workspace_repo`, `remove_workspace_repo`, `set_workspace_primary_repo` (nine call sites) | Needs a design decision plus a new test; see below |
| The docker preview-server readiness timeout never fires | `sandbox/docker/status_resolver.rs::inspect_server_container` | Tracked by the `test.fixme` on SERVER-02 in `tests/api/specs/sandbox-errors.spec.ts` |
| The GitHub preflight never runs on a restart | `crates/sandbox-control/src/auto_provision.rs::build_provision_request` runs only `preflight::preflight_validate_sandbox_control`; `RealPreflight`'s repo-access probe reaches only creation and task start | When `SandboxRuntime::ensure_running` no longer builds a plan on the reuse path; see below |
| A `moat push` commit claim is process-local | `crates/adapter-github-pr/src/api_routes.rs::claim_change`, the `GithubClaimSubject::Commit` arm — `RecentlySent::note_commit` only | When a `pull_request.synchronize` is observed reaching a replica that did not serve the claim; see below |
| Every member of a shared Sandbox runs the first member's agent harness | `sandbox/src/state.rs::register_task_context` — its three-field config clone leaves `agent_type` and `use_tmux` inherited from the pod, while `spawn_agent` is already called per member and `TaskContext` already holds its own `AgentHandle` | When a shared warm Sandbox needs two harnesses at once; see below |
| A Task auto-close spares for its sibling Bindings is told nothing about the PR | `crates/adapter-github-pr/src/routing.rs::close_subject`, the `siblings_count > 1` arm | When a Task bound to two pull requests is observed missing the close of one; see below |
| One clone attempt has no timeout of its own | `sandbox/src/init/git.rs::clone_checkout` — `run_command` awaits `git`/`gh` with no deadline, so `clone_with_retry`'s `CLONE_RETRY_BUDGET` bounds the retries between attempts but not an attempt | When a git host that black-holes rather than refuses is observed overrunning the pod's startup probe — the budget exists so init's own error reaches the termination log instead of a SIGKILL |
| The Adapter's own GitHub calls cannot see a Workspace's pinned credential | `crates/adapter-github-pr/src/auth.rs` — `client_for_outbound_reply`, `client_for_reaction`, `client_for_durable_work`, `client_for_live_status` each build an `Actor` from a login or a `TaskBinding` and never call `Actor::on_workspace`; the typed routes in `api_routes.rs` do | When a Workspace pinning a git-host credential is observed failing an event-driven reply or reaction on a repository the acting identity's own token cannot reach; see below |
| A pinned Gitness branch listing reads its base URL off the caller | `crates/workspaces/src/repository/handlers.rs::list_branches`, the `RepositoryProvider::Gitness` arm — `find_provider_token_full(&ctx.user_id, "gitness")` for `base_url`, while the token beside it may have come from the Workspace pin | When a Workspace pins a Gitness credential; see below |
| A replayed `thread.create` is refused rather than recognised | `crates/backend/src/t3code/backend.rs::free_thread_id` — a bootstrap frame re-sent after a reconnect names a thread that was created successfully, and the id is refused as taken. `send_turn`'s `already_sent` guard is the shape a fix would take, keyed on `NewThread::first_message`'s `message_uuid` | When a reconnecting client is observed showing "That thread already exists." on a thread it can see. The refusal predates the id-reclaim change, which only made the wording specific |
| An attached terminal stream outlives the control that opened it | `crates/t3code/src/terminal.rs::attach` / `forward_attach_stream` — control is checked once when the stream is opened (`controlled_thread`), never again while it runs, so a controller who opened a shell on a Task that is then set back to private keeps reading and writing it until they detach | When a public Task set private is observed leaving a stranger's attached shell live. The next call refuses; only the held stream is stale, and re-checking it needs a per-frame gate the attach loop does not have. Owner recovery in the meantime is `POST /cleanup` (write, owner-only) |
| An isolated Fork starts from a fresh clone, not the parent's Workspace | `crates/tasks/src/task/handlers.rs::fork_task` — the `!same_sandbox` path provisions an ordinary isolated Sandbox, so the fork's pod restores its own (empty) archives at `<prefix>/<fork_task_id>/{workspace,transcripts}.tar` rather than the parent's | When a Fork not sharing its parent's Sandbox must reproduce the parent's uncommitted working tree — or when a Codex isolated Fork is observed failing `thread/fork` because the parent thread is absent from its fresh `$CODEX_HOME`. See below |

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
`list_branches` reads it from `provider_tokens`, keyed on `ctx.user_id`. So the
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
the parent's Session — Claude by truncating a copy of the transcript, Codex via
`thread/fork`. Both anchors reach the pod through `task_forks`, but the bytes the
resume reads do not: an isolated Fork provisions an ordinary Sandbox, whose init
restores `<prefix>/<fork_task_id>/{workspace,transcripts}.tar` — its own,
still-empty archives — so the pod holds a fresh clone and neither the parent's
uncommitted working tree nor its Claude transcript. A `sameSandbox` Fork has no
such gap: it shares the parent's checkout and `$CLAUDE_CONFIG_DIR`/`$CODEX_HOME`,
so both anchors resolve against files already present.

Closing it is a seed step before the Fork's Sandbox provisions: S3 `copy_object`
of the parent's two archives onto the Fork's keys (the keys are
`sandbox_key`/`transcript_key` in `crates/backend/src/sandbox/grpc_server.rs`,
both `task_id`-scoped). Two conditions gate it. First, the parent's archives must
be current — a Fork is refused while the parent has a Turn in flight
(`TaskForkService::plan`), so the parent is quiescent, but whether its last sync
flushed the working tree and the transcript depends on the sync policy, and a
`SyncWorkspaceRequest.include_transcripts` toggle would force it. Second, the
copy needs the S3 client, which lives in the backend band, not `moatless-tasks`
where `fork_task` runs — so the seed belongs on the provisioning path, keyed off
the `forked_from` origin, not in the create handler. Until then an isolated
Claude Fork degrades to a fresh Session (context lost, conversation intact) and
an isolated Codex Fork errors on `thread/fork`; `sameSandbox` is the working path.

## Test and CI structure

| Deferred | Anchor | Entry condition |
|---|---|---|
| The E2E client hand-writes routes it could generate | `tests/shared/moat-api.ts` — 65 raw `(method, path)` literals | When route drift next costs a run, or alongside any broad route change |
| `E2E_GITHUB_PAT` is unset on the repository | `tests/shared/profile.ts` gates `githubRepositories` on it | A repository-settings change; un-skips 7 tests |
| No `cargo fmt --all --check` gate | `ci.yml`; a standing diff in `adapters/providers/github_pr/api.rs` | Sweep and gate together, so the gate starts green |
| Nothing alerts when `main` goes red | `ci.yml` | Pairs with a nightly build for the image tier |
| A sandbox readiness test intermittently hangs to the CI timeout | `sandbox/src/init/replacement_test/readiness.rs::sustained_ack_failure_never_re_runs_and_aborts_on_shutdown` | When it fires a second time on `main`, or when anyone next works in `sandbox/src/init` |
| Nothing holds a removed crate edge removed | `ci.yml`; `crates/backend-core/Cargo.toml` can regain `sandbox-proto` and `crates/core/Cargo.toml` can regain `moatless-api-schemas` with no gate objecting | A `cargo metadata` check over a forbidden-edge list, sized so one line per edge is the whole cost of adding one |
| Committed OpenAPI spec and TS client lag the backend | `openapi-specs.json` and `packages/api-client/src/**` lack the `fork_task` operation (`POST /api/v1/tasks/{task_id}/fork`) the tasks routes register | Run `task generate` alongside the next change touching the API surface, or on its own when the frontend first consumes the fork endpoint. Harmless until then: `build-rust` regenerates the spec fresh on every push (the `Generate OpenAPI spec` step in `ci.yml`), so `main` stays green while nothing references the missing types |

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
| No CI job executes the db-backed integration tests | The db-tests leg is `cargo nextest run --no-run` and no job provides a database, so `crates/backend/tests/integration_*/` compiles and is never run. A change that renames a table, a payload field, a route or an event identity therefore leaves these tests asserting the old one and CI still green; the drift is only visible to whoever next points a Postgres at them. Last count: 51, measured against Postgres 16.2 on 2026-09-01 — see below | Standing up a Postgres service for the db-tests leg, which is the change that stops this recurring |
| The `AdapterNewThread` outbound capability is dead in production, live in tests | `bindings/outbound.rs::direct_send_to_rule` (sole production caller) reaching `capabilities.rs::{AdapterNewThread::send_message, binding_destination_from_new_thread}`, `registry/mod.rs::{Registry::outbound_for_new_thread_by_name, OutboundOperation::NewThread}`, both impl'd by `providers/{slack,telegram}/outbound.rs` | Is subscription-based new-thread outbound being finished or abandoned? See below |
| The sandbox images are documented as two products | `docs/docker-builds.md`, `docs/sandbox-architecture.md`, `docker-compose/README.md`, the `docker-install` and `docker-testing` skills, and the commented alternative in both `.env.example` files | Any pass over the sandbox image docs |
| Two crates mint the same WebSocket ticket | `identity/src/auth/jwt.rs::generate_ws_ticket_jwt` and `t3code/src/auth.rs::generate_ws_ticket_jwt` — same `moatless-vibe-backend/ws-ticket` issuer, same 60s TTL, same claim fields under two names (`SessionClaims`, `UiRpcSessionClaims`); they differ only in which identity type they take | Any change to the ticket's issuer, TTL or claim shape, because that change has to land in both and nothing makes the second copy visible |
| `DownloadUpload` is served for nobody the repo still builds | `sandbox.proto::BackendSandboxService.DownloadUpload` and `grpc_server.rs::download_upload` — the single-message upload RPC. No sandbox built from this tree calls it; it exists because a running pod keeps the image it was provisioned with, so pods older than `DownloadUploadChunked` still do. Removing it takes the RPC, the handler, and `DownloadUploadResponse` (the sandbox's `BackendClient::download_upload` returns that message but reassembles a chunk stream to build it) | When no sandbox pod predating `DownloadUploadChunked` can still be running — the horizon is the longest-lived task's sandbox, so a deployment-wide redeploy settles it immediately |
| The secret store holds values that are not secrets | The `secrets` table, `crates/identity/src/secret/`, `SecretService`, `/api/v1/secrets`, `moat secrets`, `apps/frontend/src/features/secrets/` — the store is scoped, keyed and delivered into sandbox runtimes, and a Secret is one flavor of what it holds. The other flavor is a **Variable**: readable, listable, non-sensitive configuration such as the Claude Code toggles in `sandbox/src/init/claude_code_settings.rs`. `Variable` is the agreed umbrella name; `Secret` stays the sensitive flavor | When a wire-contract release is already breaking `/api/v1/secrets`, or when the plain flavor grows enough surface that "secret" misleads whoever reads it next. The rename is mechanical but wide — ~204 files, a table rename, and a breaking change to both the public API and the `moat` CLI — so it is not worth a pass of its own |

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

After the `ExternalReplyService` slice, this question also gates a census row.
`bindings/outbound.rs`'s remaining `crate::state::AppState` rests on exactly two
carriers: `direct_send_to_rule` (this dead function) and the retained `&AppState`
door on `resolve_files` (one caller, `adapters/handlers.rs:171`). So the file's
row has a two-condition clearance — one layering (the `resolve_files` door dies
when `adapters/handlers.rs` ports) and one product (this decision). The product
half is the cheaper of the two: *abandoned* clears `direct_send_to_rule` by
deletion at zero layering cost, so answering it is the shortest path to
`outbound.rs` leaving the sweep.

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
| `bindings/runtime.rs` has two doors onto binding upsert | `bindings/runtime.rs::upsert_binding` (`&AppState`) delegating to `::upsert_binding_into` (`&dyn TaskBindingDao`) | Delete the `&AppState` door when `runtime.rs`'s own `&AppState` functions are ported and its nine external callers move to the port-typed core |
| `bindings/runtime.rs` keeps three provider-facing `&AppState` doors | `bindings/runtime.rs` — `parent_author_task_id` and `external_id_aliases` (delegate to `state.routing`), `upsert_binding` (delegates to `upsert_binding_into`). The routing subgraph itself is off `AppState` — `RoutingService` owns its ten ports and `route_inbound_message`/`fan_out` are methods — but these three doors keep `runtime.rs` naming `AppState`, its one remaining loops→server edge. (`resolve_chat_provider_event` was a fourth; it read only `adapter_connections` and `loops`, so C6 gave it those two ports directly and its Slack/Telegram ingress callers pass them — unblocking those two files plus `telegram/poller` in the carve, without clearing `runtime.rs`.) | Each door's callers are provider-band files that still hold `AppState`; a door closes when the provider slice that owns its callers passes the port at the call site (`state.routing`), the same move the four register/subject-router sites made in Step 5. `runtime.rs` drops `AppState` when the last of the three goes. See the 9/5 partition in `.plans/backend-ports-and-crates.md` |
| `bindings/outbound.rs` keeps an `&AppState` door onto file resolution | `bindings::outbound::resolve_files` (`&AppState`) delegating to `::resolve_files_with` (`s3`, `&S3Config`, `&dyn FileDao`) | Delete the door when its sole caller `adapters/handlers.rs:171` holds the three ports. `resolve_files_with` reads three ports for one call; if a second consumer with a different shape lands before that port move, a file-resolver service is earned then, rather than three `FromRef` bounds sprawled across a handler that also extracts its own ports |
