# moatless-sandbox-control

Token signing and verification live in `moatless_core::sandbox_jwt`.
`lifecycle::watcher::classify_observation` stays here, because it calls
`Lifecycle`. The drivers' manifest metadata records their dependency
exceptions. Dependency order lives in
[BANDS](../../tooling/arch-rules/src/band_order.rs).

## What each module owns

- HTTP routes in `routes.rs` call handlers. A handler owns no provider logic.
- `lifecycle/` owns durable sandbox intent, and every start, stop, restart,
  redeploy and remove orchestration.
- `SandboxRuntime` (`moatless_sandbox_runtime::runtime`) is the provider
  mutation port. The provider crates implement it natively.
- File handlers get a ready `SandboxClient` through `SandboxConnector`, then
  call the file and search methods directly.

Reach the workspace domain only through the ports `moatless-ports` declares:
`WorkspaceResolver` and `WorkspaceCredentialReader`. Reach identity only
through `CommitIdentity`, `HarnessCredentials`, `SecretResolver` and
`GitHostTokenIssuer`. `moatless-identity` is not a dependency here. Production
code and tests alike name only the ports. `moatless-workspaces` is a
**dev-dependency**, for the one `db-tests` plan test that assembles
`ProvisionDeps` from a pool. When you add either crate under `[dependencies]`,
you re-open the edge these ports exist to close.

The read-only seams are split by audience:

- The T3 contract ports (`SandboxObserver`, `SandboxIntentReader`,
  `SandboxStarter`, `SandboxStopper`) are declared once in `moatless-ports`
  (`sandbox` module), so `backend` and `crates/t3code` share one vocabulary.
- `SandboxConnector` is declared in `sandbox-proto` (`client` module), beside
  the `SandboxClient` it returns. `moatless-sandbox-runtime` re-exports it from
  its own `client` module.
- The provider adapters implement both, and narrow backend types to the contract
  through the backend's `t3code::projection`.
- `moatless_sandbox_runtime::status_reader`'s `SandboxStatusReader` is the rich
  read the HTTP routes need: the 13-field `SandboxStatusFullResponse`, live
  status, and log streams. Its `access` module holds only the `LogOptions`
  request struct that both reads share.

## Resolving env values and plugin skills

Env values resolve **workspace > user > global**, for configuration and for
credentials, on every channel that reaches a pod. There are two such channels:
the agent env (`secrets::resolve_task_credentials`) and the sidecar env
(`secrets::resolve_server_env`).

Both express that order structurally. Seed the map with the workspace's values,
then `or_insert` each lower source. A source added in the wrong place then
cannot outrank an older one.

The pod spec's `env_vars` do not establish that order on their own. A pod's
environment is only the starting environment, and both the per-turn agent map
and a sidecar's `server.env` overlay it. A workspace value must therefore
travel on the two channels above as well as in the plan.

Plugin skills resolve in the same order. `auto_provision` passes both the
run-as identity and `shape.workspace_id` to `plugin_deliveries`, and
`plugin::activation` ranks a Workspace record above a personal record above the
global default. Drop the workspace argument and the pod silently gets the
identity's own set, with every skill the Workspace excluded back in it. The
port therefore takes the workspace id as `&str` and not as an `Option`.

## Keeping a Workspace's env_vars configuration

A Workspace's `env_vars` are configuration, and
`secrets::is_platform_owned_key` is what holds them there. Every channel that
carries them to a pod filters with it: `secrets::resolve_workspace_env_vars`
for the agent and sidecar env, `auto_provision::parse_env_vars_from_workspace`
for the pod spec. Each channel has to filter, because no one of them is
downstream of the others.

The filter covers two families of key:

1. **Reserved runtime keys**
   (`moatless_identity::secret::models::is_reserved_key`). The plan's vars would
   otherwise redefine them outright. Both runtime builders append the plan's
   vars *after* `TASK_ID` and the auth tokens, and a later entry of the same
   name wins.
2. **Git-auth keys** (`secrets::is_git_auth_key`): the three provider-token
   slots, plus the `GH_TOKEN` and `GIT_TOKEN` aliases that `init::credentials`
   derives inside the pod.

`fill_git_host_tokens` fills a provider slot only while that slot is unset. A
workspace value there therefore suppresses the validated run-as token instead
of merely outranking it. The two aliases are read straight from the pod
environment: `gh` reads them before `hosts.yml`, and `init::git` and
`init::workspace` read them for the clone. A workspace value under either name
therefore becomes the clone and push credential.

`resolve_task_credentials` and `fill_missing_from_scoped` both re-check at
their own merges. Neither a caller that assembles `workspace_env_vars`
elsewhere nor a scoped `env` secret can re-open the hole.

A Workspace that must act as another git account has two supported paths, and
both resolve a real token. `run_as_user_id` names the identity to act as. A
pinned `workspace_credentials` row names a token directly, and the pin is the
path that reaches a repository no identity here holds a credential for.
`env_vars` is neither path.

`moatless_identity::git_host_tokens::GitHostTokens` resolves a pinned git-host
credential. Seeding it into the sandbox env is not enough on its own: the
sandbox and every backend-side call must present the same account. The
Workspace on the `Actor` (`resolve_task_actor`) is what carries the credential
there. An actor built without that Workspace resolves the identity's own
credential, so a new caller that holds a Task must name the Task's Workspace.

## Authorizing a request against the Sandbox

HTTP authorization reads the Sandbox. It does not read the Task.
`authorize_task_read`, `authorize_task_control` and `authorize_task_write` in
`handlers.rs` all work from one `TaskIntent`.

There are three grants, and the split between them is permanence:

| Grant | What it allows |
|---|---|
| read | Read the Sandbox |
| control | Operate a running Sandbox: start, stop, restart, redeploy, restart a server, and write a file into the workspace |
| write | Make a persistent change the owner would have to undo: cleanup, remove, and every server-config change |

`visibility = 'public'` grants read and control to any signed-in caller. Anyone
may operate a public Task's Sandbox, and only the owner or an admin may
reconfigure it or tear it down.

A controller can open a shell. Everything a shell reaches is therefore control,
the owner's git token included. Write is the boundary a shell cannot cross.

## Writing and reconciling sandbox intent

- `Lifecycle` is the sole writer of Sandbox *intent*. Do not add a direct
  provider mutation path from a handler, an adapter, a reaper or a watcher.
- Task controls Sandbox. `moatless-tasks` writes the `sandboxes` row when a task
  is created, and this band reads that row and states intent on it. When the row
  is missing, raise an error that names the sandbox. Do not create the row.
- Durable Sandbox state lives in the `sandboxes` table. The row holds the pod's
  shape (`branch`, `headless`, `run_as_user_id`) and its access-control pair
  (`scope`, `owner_user_id`). Every control decision reads that row: who may
  read, who may act, and what the pod looks like.
- A Sandbox whose intent sits at the epoch was never requested. It is converged
  Stopped, and it is wakeable.
- `delete_task` requires a successful `lifecycle.remove` before it deletes the
  task row. `moatless-tasks` enforces this.
- Completion facts commit before publication. The backend's
  `sandbox::grpc_server::publish_event` returns success only after this crate's
  `event_dedup` durably admits the complete payload.
- `event_dedup.rs` records admission. It does not record receipt. A failed
  admission must leave the retries real, and concurrent duplicates must retry
  rather than be acknowledged.
- Operation-generation markers guard replacement operations. A marker clear must
  compare the acknowledged generation, so a late completion cannot erase a newer
  operation.
- The reconciler owns desired-versus-actual recovery. It handles woken-ready
  pending messages, removed-intent orphans, abandoned edges, and matching
  replacement generations.
- Provision-plan construction must reject a task whose sandbox identity cannot
  write that task, before it creates any runtime resource. The check reads the
  pod's own identity, the task's `created_by`. It does not read the caller who
  started it. The pod acts as the owner, so the grant the check needs is `write`
  and not `control`. Keep the check in this provider-agnostic band, and keep it
  matched to the gRPC `ScopeContext::can_write` gate
  (`grpc_server::resolve_write`).

## Where the provider contract lives

[`crates/sandbox-runtime/AGENTS.md`](../sandbox-runtime/AGENTS.md) states the
contract every provider must satisfy, beside the traits it constrains. Each
provider crate's guide states what its own runtime adds.
