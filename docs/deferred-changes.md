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
| `SandboxWakeService`'s closed-task guard is untested | `sandbox/notify.rs::SandboxWakeService::wake` — the "a closed task does not wake" branch; only the seam (`sandbox_waker::wake_for_message`) is covered, not the wake behaviour | When the rich `Lifecycle::start` is behind a trait and `SandboxWakeService` is constructible from fakes — a start port (`SandboxStarter`, declared in `moatless-backend-core`) already exists in the floor, so this is reconciliation, not new inversion |
| `TaskReaper`'s two above-band concrete fields | `task/reaper.rs::TaskReaper::new` — `k8s: K8sClient`, `sandbox: Arc<Lifecycle>` | Step 10d — a precondition of the crate compiling, not a cleanup |
| Credential broadcast fan-out reaches `tasks` | `credentials::CredentialService::store` auto-invokes `broadcast_to_scope` | Step 10c, and it must land with the fan-out test that does not exist |
| `sync_config`'s body spans two bands | `repository/handlers.rs::sync_repo_config` runs a loop sync and a workspace sync | Step 10g, as a server-band composition |
| The error-type knot | `error.rs` split, 7c's registry inversion, 8b's remainder, 8d's error half | One knot, one entry condition, recorded in the plan |
| `AuthUser` placement | `extractors.rs::AuthUser` — Step 5's open question, recorded as 8g | Recorded in the plan |
| `SetupRequirementCheck` is a single-implementation trait | `setup/evaluator.rs::SetupRequirementCheck` — `checks()` returns `vec![Box::new(GithubTokenRequirement)]`; recorded as 8h | When it earns a second implementation, or is collapsed into `evaluate_setup` |

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
up by a test fake: it is the `tasks`/`sandbox-control` crate boundary itself. Its
body needs `Lifecycle` and `NotifyPort` (sandbox-control, band 6) while every
caller sits at band 5 or above, so no placement of the code makes the reference
point downward — the trait is the only thing that does. Collapsing it back into a
function restores a `5 → 6` edge and, at the loops-band call site, restores
`loops`' dependency on `sandbox-control` outright. The reasoning, its
discriminator against `SetupRequirementCheck`, and the assertions that pin it are
in `.plans/backend-ports-and-crates.md` under Step 5.

## Correctness

Two defects found by reading, both real, neither urgent enough to have been fixed
in the change that found them.

| Deferred | Anchor | Entry condition |
|---|---|---|
| Control-scoped writes discard the bool that denies them | `workspace/handlers.rs` — `create_workspace`, `update_workspace`, `override_workspace`, `add_workspace_repo`, `remove_workspace_repo`, `set_workspace_primary_repo` (nine call sites) | Needs a design decision plus a new test; see below |
| The docker preview-server readiness timeout never fires | `sandbox/docker/status_resolver.rs::inspect_server_container` | Tracked by the `test.fixme` on SERVER-02 in `tests/api/specs/sandbox-errors.spec.ts` |

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

## Test and CI structure

| Deferred | Anchor | Entry condition |
|---|---|---|
| The E2E client hand-writes routes it could generate | `tests/shared/moat-api.ts` — 65 raw `(method, path)` literals | When route drift next costs a run, or alongside any broad route change |
| `E2E_GITHUB_PAT` is unset on the repository | `tests/shared/profile.ts` gates `githubRepositories` on it | A repository-settings change; un-skips 7 tests |
| No `cargo fmt --all --check` gate | `ci.yml`; a standing diff in `adapters/providers/github_pr/api.rs` | Sweep and gate together, so the gate starts green |
| Nothing alerts when `main` goes red | `ci.yml` | Pairs with a nightly build for the image tier |
| A sandbox readiness test intermittently hangs to the CI timeout | `sandbox/src/init/replacement_test/readiness.rs::sustained_ack_failure_never_re_runs_and_aborts_on_shutdown` | When it fires a second time on `main`, or when anyone next works in `sandbox/src/init` |

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
| `MOATLESS_DEFAULT_RUN_AS_USER_ID` is read by nothing | `config.rs::default_run_as_user_id` — declared, populated from env, never consumed | Decide whether it is an unfinished feature or removable |
| The `AdapterNewThread` outbound capability is dead in production, live in tests | `bindings/outbound.rs::direct_send_to_rule` (sole production caller) reaching `capabilities.rs::{AdapterNewThread::send_message, binding_destination_from_new_thread}`, `registry/mod.rs::{Registry::outbound_for_new_thread_by_name, OutboundOperation::NewThread}`, both impl'd by `providers/{slack,telegram}/outbound.rs` | Is subscription-based new-thread outbound being finished or abandoned? See below |
| The sandbox images are documented as two products | `docs/docker-builds.md`, `docs/sandbox-architecture.md`, `docker-compose/README.md`, the `docker-install` and `docker-testing` skills, and the commented alternative in both `.env.example` files | Any pass over the sandbox image docs |

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
| `PluginDao::clear_activation` takes an owned skill name | `crates/workspaces/src/plugin/dao.rs::PluginDao::clear_activation` — `skill_name: Option<String>` | Nothing requires the owned form; narrow it to `Option<&str>` when this signature is being changed for another reason, rather than touching a request path for that alone |
| `bindings/runtime.rs` has two doors onto binding upsert | `bindings/runtime.rs::upsert_binding` (`&AppState`) delegating to `::upsert_binding_into` (`&dyn TaskBindingDao`) | Delete the `&AppState` door when `runtime.rs`'s own `&AppState` functions are ported and its nine external callers move to the port-typed core |
| `bindings/runtime.rs` keeps four provider-facing `&AppState` doors | `bindings/runtime.rs` — `resolve_chat_provider_event` (reads two ports directly), `parent_author_task_id` and `external_id_aliases` (delegate to `state.routing`), `upsert_binding` (delegates to `upsert_binding_into`). The routing subgraph itself is off `AppState` — `RoutingService` owns its ten ports and `route_inbound_message`/`fan_out` are methods — but these four doors keep `runtime.rs` naming `AppState`, its one remaining loops→server edge | Each door's callers are provider-band files that still hold `AppState`; a door closes when the provider slice that owns its callers passes the port at the call site (`state.routing`), the same move the four register/subject-router sites made in Step 5. `runtime.rs` drops `AppState` when the last of the four goes. See the 9/5 partition in `.plans/backend-ports-and-crates.md` |
| `sandbox/notify.rs` has two doors onto the sandbox wake | `sandbox::notify::ensure_sandbox_and_notify` (`&AppState`) delegating to `SandboxWaker::wake` on `state.sandbox_waker` | Delete the `&AppState` door when its four remaining callers (linear ×3, `event_handler/agent_handlers.rs`) hold the wake port directly; each is above the sandbox band, so all four reach it downward-legally today |
| `adapters/common.rs` has two doors onto task-access auth | `adapters::common::ensure_task_access` (`&AppState`) delegating to `::ensure_task_access_with` (`&dyn TaskDao`) | Delete the `&AppState` door when its eight remaining callers across three files — `adapters/handlers.rs` ×2, `providers/github_pr/api_routes.rs` ×4, `providers/linear/api_routes.rs` ×2 — hold the task DAO at the call site; `bindings/{handlers,reply}.rs` already moved onto the core |
| `bindings/outbound.rs` keeps an `&AppState` door onto file resolution | `bindings::outbound::resolve_files` (`&AppState`) delegating to `::resolve_files_with` (`s3`, `&S3Config`, `&dyn FileDao`) | Delete the door when its sole caller `adapters/handlers.rs:171` holds the three ports. `resolve_files_with` reads three ports for one call; if a second consumer with a different shape lands before that port move, a file-resolver service is earned then, rather than three `FromRef` bounds sprawled across a handler that also extracts its own ports |
