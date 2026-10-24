---
name: shadow-dev
description: "Run and rebuild the Moatless backend inside a sandbox using scripts/shadow. Use for a local shadow backend against shared development services, run as a Script."
---

# Shadow development

The shadow runs `moatless-backend` on port 8080 through
[up.sh](../../../scripts/shadow/up.sh). It shares development Postgres and
object storage. Use a dedicated workspace so normal repository tasks do not start
an extra backend or inherit its elevated service account.

## Workspace setup

Read [shadow configuration](../../../scripts/shadow/workspaces.shadow.json) and
[the shadow runbook](../../../scripts/shadow/README.md), then declare a
Workspace entry in `.moatless/workspaces.json` with the repository and the
backend Script. Run configuration
belongs to the workspace, not the repository git record.

- Backend Script: name `backend`, command `bash scripts/shadow/up.sh`, port 8080,
  `runOnStart: true` so it starts as soon as the sandbox is ready.
- Set `SHADOW_CARGO_PROFILE=dev` in the backend Script's `envVars` and in the shell
  running rebuilds. Both scripts default to release; mismatching profiles makes
  the Script start a different binary from the one rebuilt.
- Set resources in the Workspace's `.moatless/workspaces.json` entry, for example
  `"resources": {"storage":"40Gi","pvcEnabled":false,"s3Enabled":true}` when
  snapshot-backed ephemeral storage matches the cluster. Check available disk
  before compiling.
- Image selection is the entry's `dockerImage`. The optional dev image comes from
  [Dockerfile.sandbox-dev](../../../docker/Dockerfile.sandbox-dev).
- `sync-env.sh` needs access to the source backend deployment and Secrets.
  Provisioning child sandboxes additionally needs the chart's `shadowDev` RBAC
  and the configured workspace service account. Verify the actual permissions.

```bash
moat tasks create --workspace <shadow-workspace-id> --agent-profile claude-code \
  --headless false --name "Shadow development"
moat tasks status <task-id> --json
moat tasks scripts list <task-id>
curl --fail --max-time 10 http://localhost:8080/health
```

`moat tasks logs` streams the sandbox container's own logs, not the Script's —
the `backend` Script's output is in its own terminal, read in the T3 UI (attach
or restart it with `moat tasks scripts run backend`). Use the T3 UI's preview
tab for the backend's published URL under Kubernetes; do not derive it from task
ID, since shared sandbox tasks can have a different sandbox identity.
Authenticated API checks need the API key even when `/health` succeeds publicly.

## Rebuild and environment

Confirm the backend Script exists first — `moat tasks scripts list <task-id>`.
`rebuild.sh` builds `moatless-backend` and only then bumps the trigger that
`up.sh` watches, and `up.sh` runs only where the backend Script does. With none,
the whole build is spent and nothing consumes the trigger.

```bash
export SHADOW_CARGO_PROFILE=dev
bash scripts/shadow/rebuild.sh
curl --fail --max-time 10 http://localhost:8080/health
```

Read bounded backend logs for build errors; run
`cargo build -p moatless-backend` for a direct dev build if needed. Follow root
Rust verification commands before calling a code change complete.

[sync-env.sh](../../../scripts/shadow/sync-env.sh) imports deployment configuration
and applies the isolation settings: disabled event handler, idle reaper,
reconciliation actions, cron scheduler, Codex refresh, and migrations. It also
adds localhost/127.0.0.1 port 5733 browser origins.
These settings do not make shared database writes private.

Additional overrides go in `$RUNTIME_DIR/shadow/overrides.env`, and nowhere else,
as sourceable `export KEY=value` lines. Resync/rebuild after changing environment.
Use a separate database before enabling migrations for a schema experiment. Do not
repair workspace service-account configuration by updating repository columns in SQL.

For the T3 Code UI against this backend, read [t3-development](../t3-development/SKILL.md).
