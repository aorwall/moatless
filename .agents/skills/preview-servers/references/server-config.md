# Server configuration

Read [ServerConfig and ResourceConfig](../../../../crates/api-schemas/src/workspace.rs)
for field types and defaults. `name` identifies a server; `port` is optional in the
schema, so read the resolved port and URL from task status. `startCommands`,
`envVars`, `startupTimeoutSeconds`, image, and per-server resources configure its
container. Bind the application to `0.0.0.0` on the resolved port.

`startupTimeoutSeconds` controls health classification, not Kubernetes probe
configuration. `autoReload` informs the agent prompt; the application supplies
its own file watcher. Only requests/limits in server `resources` apply to that
container; workspace storage is configured on the workspace.

## Environment

The container's server-specific environment is the base. `server.env` supplies
user secrets and is refreshed periodically (15 minutes); live overrides apply
last. A restart reads the files currently present, but cannot accelerate a secret
refresh that has not happened. Container env changes require redeploying.
Workspace-level `envVars` are injected into the sandbox, not preview containers;
put server variables in the server's own `envVars` or a live override.

## Resolve and promote

```bash
moat tasks get <task-id> --json | jq '{workspaceId, sandboxId}'
moat workspaces get <workspace-id>
moat tasks servers <task-id> list
```

Do not derive the workspace from repository ID. Read its source: git-owned
workspaces are declared in `.moatless/workspaces.json`;
manual workspaces use `moat workspaces update`. `--servers-json` replaces the whole
array, so include every server. `moat workspaces override` takes manual ownership;
`reset-git` restores git configuration. Explicit repository config sync can reclaim
overrides, so do not use it as an unrelated repair.

Live overrides are stored under the shared workspace's `.moatless/servers/<name>/`:
`override.json`, derived `command` and `env`, and the `generation` restart counter.
Use the CLI to author these files. They affect every task sharing that sandbox.
A stopped sandbox reports overrides as unknown, which does not mean absent.

Overrides survive only as far as workspace persistence does. See the independent
PVC/S3 switches in [resources and recovery](../../sandbox-troubleshooting/references/failure-modes.md).
An S3 restore recovers the last snapshot, not necessarily the latest override.
Promote a verified live change to its configuration owner, then verify it in a
new or redeployed sandbox using the returned preview URL.
