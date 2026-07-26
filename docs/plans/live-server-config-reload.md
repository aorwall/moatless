# Live Preview-Server Config Reload

> **Status: implemented.** The design below is what shipped, except where the
> "As built" notes say otherwise. Open questions at the end are still open.

## Summary

Today the only supported way to change a preview server's configuration on a
running task is `POST /api/sandbox/v1/tasks/{id}/redeploy`, which rebuilds the
whole pod — the agent container included — and is explicitly blocked from inside
the sandbox. There is no "change the start command and restart just this server"
path, from the CLI, the API, or the agent.

This plan proposes one: a **generation-file control channel** on the shared data
volume plus a supervisor wrapper around each server sidecar, so a server can be
reconfigured and restarted in place in seconds without touching the pod spec,
the PVC, or the agent session.

Target workflow (all of it usable *from inside* the sandbox):

```bash
moat tasks servers $TASK_ID list
moat tasks servers $TASK_ID set dev --start-command "bun run dev -- --host 0.0.0.0 --port 3000"
moat tasks servers $TASK_ID set dev --env VITE_REMOTE_API=http://localhost:8080 --env LOG_LEVEL=debug
moat tasks servers $TASK_ID set dev --unset LOG_LEVEL
moat tasks servers $TASK_ID restart dev
moat tasks servers $TASK_ID reset dev
moat tasks logs    $TASK_ID --server dev
```

## Current State

### Where server config lives

| Layer | Location |
|---|---|
| Git (optional) | `.moatless/workspaces.json` → synced into the DB |
| DB (source of truth) | `workspaces.servers` JSONB |
| Per-task override | `tasks.disabled_servers` JSONB (crash-loop auto-disable only) |
| Pod spec | one sidecar container per `ServerConfig`, built at provision time |

`ServerConfig` (`crates/api-schemas/src/repository.rs:89`, mirrored in
`backend/src/sandbox/models.rs:90`): `name, label, port, start_commands,
startup_timeout_seconds, auto_reload, enable_inspector, default, docker_image,
env_vars, resources`.

### How a server runs

Servers are **not** supervised by the sandbox process — `sandbox/src/grpc_server.rs:1`
says so outright ("Server management is handled by K8s (separate containers), not
by the sandbox"). `build_server_containers`
(`backend/src/sandbox/k8s/pod_builder.rs:1002`) emits one container per server,
named `{name}-server`, whose command is a one-shot shell string:

```
until [ -f /opt/moatless/data/.workspace-ready ]; do sleep 2; done &&
if [ -f /opt/moatless/data/server.env ]; then set -a && . /opt/moatless/data/server.env && set +a; fi &&
cd "${WORKSPACE_PATH:-/opt/moatless/repo}" && <joined start_commands>
```

The sidecar shares three mounts with the sandbox container: `/opt/moatless`
(workspace PVC), `/opt/moatless/data` (shared-data), `/opt/moatless/build-cache`.
Docker mode builds the same shape in
`backend/src/sandbox/docker/container_builder.rs:337`.

### Where a server's env comes from

Three sources, three different delivery mechanisms — this matters because only
one of them is currently mutable without a pod rebuild:

| Source | Set via | Lands in | Reaches the sidecar? |
|---|---|---|---|
| Workspace `envVars` | `moat workspaces update --env-vars-json` | **sandbox container env only** (`pod_builder.rs:785`) | **No** |
| Per-server `envVars` | `servers[].envVars` in the workspace config | server container env (`pod_builder.rs:1041`) | Yes, baked in the pod spec |
| User secrets | `moat secret set` | `server.env`, sourced by the wrapper (`sandbox/src/init/server_env.rs`) | Yes, re-read on every sidecar start |
| Runtime | platform | `PORT`, `WORKSPACE_PATH`, `DATA_DIR` on the container; `WORKSPACE_PATH` overridden via `server.env` (`credentials.rs:208`) | Yes |

Two consequences worth calling out:

- **Workspace-level `envVars` silently never reach preview servers.** Verified
  here: `CARGO_BUILD_JOBS=2` is set in the sandbox container and is not passed to
  server containers at all. Anyone setting a workspace env var expecting the dev
  server to see it gets no error and no effect.
- **Secrets are already almost live.** The credential refresher rewrites
  `server.env` every 15 minutes (`sandbox/src/init/refresher.rs`), and the
  wrapper re-sources it on each start — so `moat secret set` + a server restart
  is a working live-update path *the moment a restart primitive exists*.

### What propagates live today, and what does not

| Change | Applies without a pod rebuild? |
|---|---|
| Repo file contents (incl. `package.json` scripts) | On next sidecar start — the repo is a shared mount |
| `server.env` (secrets, `WORKSPACE_PATH`) | On next sidecar start — sourced by the wrapper each time |
| Secrets (`moat secret set`) | Into `server.env` within 15 min; effective on next sidecar start |
| `start_commands` | **No** — baked into the pod spec |
| `port`, `docker_image`, `resources`, `env_vars` | **No** — baked into the pod spec (and port also drives the Service/IngressRoute) |

`PATCH /api/v1/workspaces/{id}` writes the DB row and nothing else
(`backend/src/workspace/handlers.rs:277`). `restart` reuses the existing pod
template; only `redeploy` rebuilds it from fresh DB state
(`backend/src/sandbox/k8s/deployment.rs:322`).

### What the CLI exposes

`moat tasks status | logs --server | preview | restart | redeploy`. `restart` and
`redeploy` both hard-fail when `config.is_sandbox()`
(`crates/cli/src/commands/tasks.rs:338,349`) — correctly, since both would kill
the agent issuing the command. There is no per-server verb at all.

## Measured Behaviour (live experiment, task `cba6a9e8`, K8s)

Run against this sandbox's `dev` server (Vite on :3000) to establish what the
runtime actually permits:

1. **The sandbox cannot reach the control plane directly.** No shared PID
   namespace (`ps` shows only sandbox-container processes, so the server process
   cannot be signalled), and requests to `https://kubernetes.default.svc` time
   out — the pod's ServiceAccount token is mounted but the API server is
   unreachable by network policy. Any restart trigger must go through the
   backend or through the shared filesystem.

2. **`/opt/moatless/data` is a working bidirectional channel.** Files written
   from the sandbox container appear to the sidecar. `server.env` already uses
   this: `WORKSPACE_PATH` is *not* the pod-spec value `/opt/moatless/repo` (that
   path does not exist) but `/opt/moatless/workspace/moatless`, injected by the
   sandbox at `sandbox/src/init/credentials.rs:208` and picked up when the
   sidecar sources the file.

3. **A sidecar restart is cheap, in-place, and invisible to the agent.** Edited
   the root `package.json` `dev` script to prepend a marker, then forced the Vite
   process to exit. Result:

   ```
   dev-server: restartCount 1, lastExitCode 0, lastTerminatedReason Completed
   sandbox:    restartCount 0                     <- agent session untouched
   ```

   and the restarted sidecar's logs show the **new** command:

   ```
   $ echo '[live-config-test] start command v2 in effect' && cd apps/frontend && bun dev --host "0.0.0.0"
   [live-config-test] start command v2 in effect
   VITE v7.2.2  ready in 1625 ms
   ```

   Whole cycle ≈ 5 seconds. No pod recreation, no PVC detach, no agent restart.

**Conclusion.** The mechanism we need already works at the container level; what
is missing is (a) a supported way to express the new config and (b) a supported
way to trigger the restart. The only trigger available today is "make the server
process exit", which is both a hack and actively harmful: every container exit
increments `restartCount`, and at `SANDBOX_MAX_CRASH_RESTART_COUNT` (default 5)
the pod watcher auto-disables the server and forces a redeploy
(`backend/src/sandbox/k8s/pod_watcher.rs:1313`). Five manual restarts and the
preview disappears.

## Design

### Control channel

Files under the already-shared data volume:

```
/opt/moatless/data/servers/<name>/command      # optional start-command override
/opt/moatless/data/servers/<name>/env          # optional env override (shell fragment, 0600)
/opt/moatless/data/servers/<name>/generation   # monotonic counter; a bump means "restart"
/opt/moatless/data/server.env                  # unchanged, already exists
```

Written atomically (tmp + rename) by the sandbox; read by the sidecar. Absent
files mean "use what the pod spec baked in", so the control channel is pure
overlay and the pod spec stays the default.

The env override is a shell fragment rather than a map, so that *unsetting* a
variable the pod spec injected is expressible — sourcing can add but never
remove:

```sh
export VITE_REMOTE_API='http://localhost:8080'
unset LOG_LEVEL
```

**Env precedence** (later wins, all resolved at child start so every restart
re-reads them):

1. container env — `PORT`, `WORKSPACE_PATH`, `DATA_DIR`, per-server `envVars`
2. `server.env` — user secrets + the real `WORKSPACE_PATH`
3. `servers/<name>/env` — the live override

Values are shell-quoted with the existing `shell_single_quote`
(`sandbox/src/init/server_env.rs`), the file is `0600` like `server.env`, and
values are never logged. Keys are validated with `is_valid_secret_key`
(`^[A-Z_][A-Z0-9_]*$`) and rejected if in `RESERVED_SECRET_KEYS`
(`backend/src/secret/models.rs:122`) or in a small server-runtime set —
`PORT`, `WORKSPACE_PATH`, `DATA_DIR` — which the platform owns.

**`--env` is for configuration, not secrets.** Override values are stored as
plaintext JSONB (same exposure as today's `workspaces.env_vars`). Real secrets
keep going through `moat secret set`, which is encrypted at rest and, per the
table above, becomes a live path as soon as restart exists.

### Sidecar supervisor wrapper

Replace the one-shot command in `pod_builder.rs` / `container_builder.rs` with a
loop. Bash only — server containers may use an arbitrary `docker_image`, so we
cannot assume the sandbox binary is present.

```bash
set -m                                   # each child gets its own process group
CTL=/opt/moatless/data/servers/<name>
DEFAULT_CMD='<joined start_commands>'
until [ -f /opt/moatless/data/.workspace-ready ]; do sleep 2; done
while :; do
  gen=$(cat "$CTL/generation" 2>/dev/null || echo 0)
  cmd=$(cat "$CTL/command" 2>/dev/null); [ -n "$cmd" ] || cmd=$DEFAULT_CMD
  ( [ -f /opt/moatless/data/server.env ] && { set -a; . /opt/moatless/data/server.env; set +a; }
    [ -f "$CTL/env" ] && { set -a; . "$CTL/env"; set +a; }
    cd "${WORKSPACE_PATH:-/opt/moatless/repo}"; exec bash -c "$cmd" ) &
  child=$!
  while kill -0 "$child" 2>/dev/null; do
    [ "$(cat "$CTL/generation" 2>/dev/null || echo 0)" = "$gen" ] || break
    sleep 1
  done
  if kill -0 "$child" 2>/dev/null; then          # generation bumped -> restart in place
    kill -TERM -"$child" 2>/dev/null
    ( sleep 10; kill -KILL -"$child" 2>/dev/null ) &
    wait "$child" 2>/dev/null; continue
  fi
  wait "$child"; exit $?                         # died on its own -> exit, as today
done
```

**Invariant: only a generation bump restarts in place.** A server that crashes
still exits the container, so readiness-timeout detection and crash-loop
auto-disable behave exactly as they do now, and user-initiated restarts do not
consume the crash budget.

### Transport

New RPC on the existing backend→sandbox `SandboxService`
(`crates/sandbox-proto/proto/sandbox/v1/sandbox.proto:294`):

```proto
rpc ApplyServerConfig(ApplyServerConfigRequest) returns (ApplyServerConfigResponse);

message ApplyServerConfigRequest {
  string task_id = 1;
  string server_name = 2;
  // The complete override state, not a delta — the backend owns the merge
  optional string start_command = 3;
  map<string, string> env = 4;
  repeated string unset_env = 5;
  bool restart = 6;
}
message ApplyServerConfigResponse { uint64 generation = 1; }
```

The sandbox handler writes the control files and bumps the generation. Nothing
about it is K8s-specific, so Docker mode gets the same behaviour for free.

**As built:** the plan's earlier draft had a `replace_env` flag so the sandbox
could merge deltas. Dropped — the RPC carries whole state and the backend, which
already owns the persisted copy, does the merging. One merge implementation, and
the persisted row and the control files cannot drift.

### Persistence

New `tasks.server_overrides` JSONB, mirroring `tasks.disabled_servers`
(migration `0061`):

```json
{ "dev": { "startCommand": "bun run dev -- --host 0.0.0.0",
           "env": { "VITE_REMOTE_API": "http://localhost:8080" },
           "unsetEnv": ["LOG_LEVEL"] } }
```

Layered in `auto_provision.rs` next to `filter_disabled_servers`, and
re-materialized into the control files by the sandbox at init, so an override
survives stop/start and redeploy without being baked into the pod spec.

### API and CLI

| Verb | Endpoint | Behaviour |
|---|---|---|
| restart | `POST .../tasks/{id}/servers/{name}/restart` | bump generation only |
| set | `PATCH .../tasks/{id}/servers/{name}` | persist override, push, restart |
| reset | `DELETE .../tasks/{id}/servers/{name}/override` | drop the override, restart on workspace config |

`PATCH` body:

```json
{ "startCommands": ["bun run dev -- --host 0.0.0.0"],
  "env": { "VITE_REMOTE_API": "http://localhost:8080" },
  "unsetEnv": ["LOG_LEVEL"],
  "restart": true }
```

Fields that genuinely cannot be applied live — `port` (drives Service +
IngressRoute) and `dockerImage` (the container's identity) — are accepted by the
request schema only so that sending them returns `409` rather than silently
doing nothing.

CLI: `moat tasks servers <task_id> list|set|restart|reset`, **allowed from inside
the sandbox** (unlike `tasks restart`/`redeploy`) because none of it touches the
agent container.

**As built:**

- `resources` and `enableInspector` are not in the request schema at all. They
  are pod-spec fields with no live path, and adding them just to reject them
  buys nothing — an unknown field is already a clear error.
- `servers list` reads `GET .../server/status`, which now carries each server's
  `configOverride` alongside its status. Override values are printed as-is: they
  are configuration, not secrets (see the note above), so masking them would only
  make the command less useful.

## Change List

| Layer | Work |
|---|---|
| `crates/sandbox-proto` | `ApplyServerConfig` RPC + messages |
| `sandbox/src` | control-file writer (atomic, `0600`, reuses `shell_single_quote`), gRPC handler, init materialization |
| `backend/src/sandbox/server_runner.rs` | supervisor wrapper, shared by both builders |
| `backend/src/sandbox/k8s/pod_builder.rs` | use the wrapper; pass overrides in `REPO_CONFIG` |
| `backend/src/sandbox/docker/container_builder.rs` | same |
| `backend/migrations` | `0098_task_server_overrides.sql` — `tasks.server_overrides` JSONB |
| `backend/src/sandbox` | 3 handlers, routes, DTOs, client method, override layering in `auto_provision.rs` |
| `crates/api-schemas`, `crates/api-client` | DTO mirrors + 4 client methods |
| `crates/cli` | `moat tasks servers` subcommand |
| `apps/frontend` | not done — "Restart server" in the preview panel is still open |
| `specs/sandbox` | SERVER-03..09 |

No breaking API change: every new field is additive and optional.

## Scenarios

Written up in `specs/sandbox/01-sandbox-scenarios.md`:

- **SERVER-03** — a start-command override restarts the server in place: the
  sandbox container is untouched and no pod or container is replaced.
- **SERVER-04** — the override survives a sandbox stop/start.
- **SERVER-05** — `--env` takes effect in the server process and `--unset`
  removes a variable the container spec injected.
- **SERVER-06** — changing `port` or `dockerImage` is rejected as requiring a
  redeploy.
- **SERVER-07** — `reset` drops the override and restarts on workspace config.
- **SERVER-08** — a reserved env key is rejected and nothing is persisted.
- **SERVER-09** — a server that crashes on its own still exits the container and
  is still auto-disabled at the crash-loop threshold.

## Interim Workflow (superseded)

Kept as a record of what the workflow was before this shipped, and of what
`redeploy` still costs when a change genuinely needs a pod rebuild (port, image,
resources).

From **outside** the sandbox:

```bash
moat workspaces override <ws-id> --servers-json '[{"name":"dev", ...}]'   # or edit .moatless/workspaces.json + push
moat tasks redeploy $TASK_ID
```

Costs a full pod rebuild (~1–2 min) and restarts the agent. Env vars go in the
same JSON, under the server's `envVars` — note that the workspace-level
`envVars` field will *not* reach the server (see the env-source table).

From **inside** the sandbox there is no supported path; the only lever is
changing what the start command resolves to (e.g. a `package.json` script or a
`.env` file the framework reads) and waiting for the next sidecar restart.

## Open Questions

1. **Scope of `set`** — shipped task-level only, which keeps the blast radius to
   one sandbox. Still open: promoting a proven override to the workspace, e.g. a
   `--workspace` flag that writes through to `workspaces.servers`. Until then the
   promotion path is `moat workspaces override` by hand.
2. **Workspace `envVars` never reaching sidecars** — a pre-existing silent gap
   (see the env-source table). Fixing it means either passing `plan.env_vars`
   into the server containers or folding them into `server.env`. It is a
   behaviour change for existing tasks — servers would suddenly see variables
   they never had — so it is listed separately rather than assumed. Recommend
   folding it in: the env story is incoherent without it, and `server.env` is
   the delivery path that is already live-refreshed.
3. **Agent access** — shipped with full access: all four verbs authorize through
   `authorize_task_control`, the same check as every other task-control endpoint,
   so an agent can `set` on its own task. The narrower alternative (restart-only
   from inside the sandbox) is still available if this proves too permissive, but
   nothing here escapes the task's own sandbox.
</content>
</invoke>
