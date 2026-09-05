# moatless-sandbox-control

Durable sandbox lifecycle intent, reconciliation, ingress, secrets, and preflight.
Runtime contracts and observation publishing live in `moatless-sandbox-runtime`;
token signing and verification live in `moatless_core::sandbox_jwt`.
`lifecycle::watcher::classify_observation` stays here because it calls `Lifecycle`.
The drivers' manifest metadata records their dependency exceptions.
Dependency order lives in [BANDS](../../tooling/arch-rules/src/band_order.rs).

## Ownership Map

- HTTP routes in `routes.rs` call handlers; handlers do not own provider logic.
- `lifecycle/` owns durable sandbox intent and all
  start/stop/restart/redeploy/remove orchestration.
- `SandboxRuntime` (`moatless_sandbox_runtime::runtime`) is the provider
  mutation port. The provider crates implement it natively and return
  `RuntimeOutcome`; provider adapters do not publish lifecycle events.
- The workspace domain is reached only through the ports
  `moatless-ports` declares (`WorkspaceResolver`,
  `WorkspaceCredentialReader`), and identity only through `CommitIdentity`,
  `HarnessCredentials`, `SecretResolver` and `GitHostTokenIssuer`.
  `moatless-identity` is not a dependency here at all — production and tests
  alike name only the ports. `moatless-workspaces` is a **dev-dependency**, for
  the one `db-tests` plan test that assembles `ProvisionDeps` from a pool. Adding
  either under `[dependencies]` re-opens the edge these ports exist to close.
- The read-only seams are split by audience. The T3 contract ports
  (`SandboxObserver`, `SandboxIntentReader`, `SandboxStarter`,
  `SandboxStopper`) are declared once in `moatless-ports` (`sandbox`
  module) so `backend` and `crates/t3code` share one vocabulary;
  `SandboxConnector` is declared in `sandbox-proto` (`client` module) beside the
  `SandboxClient` it hands out, and `moatless-sandbox-runtime` re-exports it
  from its `client` module. The provider adapters implement them and narrow
  backend types to the contract through the backend's `t3code::projection`.
  `moatless_sandbox_runtime::status_reader`'s `SandboxStatusReader` is the rich
  read (the 13-field `SandboxStatusFullResponse`, live status, and log streams)
  the HTTP routes need. Its `access` module holds only the `LogOptions` request
  struct shared by both.
- File handlers get a ready `SandboxClient` through `SandboxConnector`, then call
  file/search methods directly.
- The unified task reaper lives in `moatless-backend`
  (`crates/server/src/reaper.rs`); task-scoped stop/remove still routes
  through `Lifecycle` here.

## Load-Bearing Rules

- Env values resolve **workspace > user > global**, for configuration and
  credentials alike, on every channel that reaches a pod: the agent env
  (`secrets::resolve_task_credentials`) and the sidecar env
  (`secrets::resolve_server_env`). Both express it structurally — seed the map
  with the workspace's values, then `or_insert` each lower source — so a new
  source cannot outrank an older one by being added in the wrong place. The
  pod spec's `env_vars` do not establish that order on their own: a pod's
  environment is only the starting environment, and both the per-turn agent map
  and a sidecar's `server.env` overlay it. A workspace value must therefore
  travel on those channels too, not only in the plan.
- Plugin skills resolve in that same order. `auto_provision` passes both the
  run-as identity and `shape.workspace_id` to `plugin_deliveries`, and
  `plugin::activation` ranks a Workspace record above a personal one above the
  global default. Dropping the workspace argument would silently hand a pod the
  identity's own set — every skill the Workspace excluded, back again — which is
  why the port takes it as `&str` rather than an `Option`.
- A Workspace's `env_vars` are configuration, and `secrets::is_platform_owned_key`
  is what keeps them from becoming something else. Every channel that carries
  them to a pod filters with it — `secrets::resolve_workspace_env_vars` for the
  agent and sidecar env, `auto_provision::parse_env_vars_from_workspace` for the
  pod spec — and each has to, because none of them is downstream of the others.
  Two families of key: reserved runtime keys
  (`moatless_identity::secret::models::is_reserved_key`), which the plan's vars
  would otherwise redefine outright since both runtime builders append them
  *after* `TASK_ID` and the auth tokens and a later entry of the same name wins;
  and every git-auth key (`secrets::is_git_auth_key`) — the three provider-token
  slots plus the `GH_TOKEN`/`GIT_TOKEN` aliases `init::credentials` derives
  inside the pod. `fill_git_host_tokens` fills a provider slot only when still
  unset, so a workspace value there would suppress the validated run-as token
  rather than merely outrank it; and the aliases are read straight from the pod
  environment (`gh` before `hosts.yml`, `init::git`/`init::workspace` for
  clone), so a workspace value under either name is used as the clone/push
  credential. `resolve_task_credentials` and `fill_missing_from_scoped` both
  re-check at their own merges, so neither a caller assembling
  `workspace_env_vars` from elsewhere nor a scoped `env` secret can reopen it.
  A Workspace that should act as another git account has two supported paths,
  and both resolve a real token: `run_as_user_id` names the identity to act as,
  and a pinned `workspace_credentials` row names a token directly. A pin is the
  one that reaches a repository no identity here holds a credential for. Neither
  is `env_vars`.
- A pinned git-host credential is resolved by
  `moatless_identity::git_host_tokens::GitHostTokens`, not only seeded into the
  sandbox env, so the sandbox and every backend-side call present the same
  account. What carries it there is the Workspace on the `Actor`
  (`resolve_task_actor`) — an actor built without it resolves the identity's own
  credential instead, so a new caller that has a Task in hand must name its
  Workspace.
- `Lifecycle` is the sole writer of Sandbox *intent*. Do not add direct provider
  mutation paths from handlers, adapters, reapers, or watchers.
- Task controls Sandbox. `moatless-tasks` writes the `sandboxes` row when a task
  is created; this band reads it and states intent on it. A missing row is an
  error naming the sandbox, not a row to create.
- Durable Sandbox state lives in the `sandboxes` table, including the pod's
  shape (`branch`, `headless`, `run_as_user_id`) and its access-control pair
  (`scope`, `owner_user_id`). Every control decision — who may read, who may
  act, what the pod looks like — comes off that row. A Sandbox with intent at
  the epoch was never requested, is converged Stopped, and is wakeable.
- HTTP authorization reads the Sandbox, not the Task: `authorize_task_read` /
  `authorize_task_control` / `authorize_task_write` in `handlers.rs` work from
  one `TaskIntent`. There are three grants, and the split is by permanence:
  read, **control** (operate a running Sandbox — start, stop, restart, redeploy,
  restart a server, and write a file into the workspace), and **write** (a
  persistent change the owner would have to undo — cleanup/remove and every
  server-config change). `visibility = 'public'` grants read and control to any
  signed-in caller, so a public Task's Sandbox is one anyone may operate but
  only the owner or an admin may reconfigure or tear down. A controller can open
  a shell, so anything a shell reaches (the owner's git token included) is
  control by construction; write is only the boundary the shell cannot cross.
- `delete_task` requires successful `lifecycle.remove` before deleting the task
  row (enforced in `moatless-tasks`).
- Completion facts commit before publication. The backend's
  `sandbox::grpc_server::publish_event` returns success only after the complete
  payload is durably admitted, through this crate's `event_dedup`.
- `event_dedup.rs` records admission, not receipt. Failed admission must leave
  retries real, and concurrent duplicates should retry rather than be
  acknowledged.
- Operation-generation markers guard replacement operations. Marker clear must
  compare the acknowledged generation so late completions cannot erase newer
  operations.
- The reconciler is the desired-vs-actual recovery owner. It handles
  woken-ready pending messages, removed-intent orphans, abandoned edges, and
  matching replacement generations.
- `classify_grpc_error()` (declared in `sandbox-proto`, re-exported from
  `moatless_sandbox_runtime::client`) must retain the underlying `tonic::Status`
  as the error source while mapping client-visible status codes.
- Provision-plan construction and expected-image selection stay behind lifecycle
  and the concrete runtime adapters.
- Provision-plan construction must reject tasks whose sandbox identity cannot
  write the task before any runtime resource is created. This checks the pod's
  own identity (the task's `created_by`), not the caller who started it, so it
  is `write` and not `control`: the pod acts as the owner. The check belongs in
  this provider-agnostic band and must continue to match the gRPC
  `ScopeContext::can_write` gate (`grpc_server::resolve_write`).

## The provider contract

The contract every provider must satisfy is stated in
`crates/sandbox-runtime/AGENTS.md`, beside the traits it constrains. The runtime
realizations live in the provider crates' own guides.
