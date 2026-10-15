# Verified command workflows

The [command parsers](../../../../crates/cli/src/commands) own flags and defaults.
Use `moat <group> <command> --help` rather than copying an exhaustive catalogue.

## Response shapes

```bash
moat repos list --json | jq '.[] | {id, name}'
moat tasks list --json | jq '.items[] | {taskId, name, status}'
moat tasks get <task-id> --json | jq '{taskId, sandboxId, workspaceId}'
```

## Task lifecycle

Create with `--agent-profile` plus `--repo`, `--workspace`, or `--same-sandbox`.
`--same-sandbox` can take a task ID or use the ambient task; `--checkout-mode
shared|isolated` only applies with it. `--parent` sets a parent explicitly.
The backend sends a `created_by` parent a notice, and wakes it, each time a
child's turn completes or errors. A notice waits for the parent's active turn to
end, and the notices that land during one turn open the next together. The
notice carries no result, so a child still sends its result with `moat send
--task <parent>`, and a parent needs no reply to a notice. `moat send --task <id>
--queue` waits the same way; without `--queue` a message steers into the active
turn.

`moat tasks status <task-id> --result` reports where the work stands — working,
waiting for input, running commands, waiting for children, done — and the latest
result. `moat tasks cancel <task-id>` stops the task's and its descendants'
agents and discards the messages still waiting for them; the tasks stay open,
unlike `close`.

`moat tasks fork <task-id>` supports `--at-turn`, `--message`, and `--same-sandbox`.
Read its help before combining these. Stop/start controls the sandbox; close
controls task lifecycle. Never stop the sandbox that is running the verification
session. A shared sandbox can affect other task members.

## Configuration ownership

Repository commands configure git connection fields: name, URL, branch, provider,
kind, description, and scope as supported by each command. Workspace commands own
image, setup commands, scripts, environment, resources, language servers, and
sandbox service account.

Inspect `moat workspaces update --help` for `--scripts-json`, `--env-vars-json`,
`--resources-json`, `--language-servers`, and `--docker-image`. Get the workspace ID from the task.
For git-managed workspaces, edit `.moatless/workspaces.json` in the owning repository
or use `moat workspaces override` to take manual ownership. `reset-git` returns
ownership to git. Explicit `moat repos sync-config` can reclaim overridden
workspaces; inspect its effects before using it as a repair.

Secrets use the singular `moat secret` group. Automation uses `moat loops`.

## Diagnostics

```bash
moat tasks diagnose <task-id>
moat tasks status <task-id>
moat events list <task-id> --errors
moat messages list <task-id> --full
moat tasks logs <task-id> --no-follow --tail 500
moat tasks scripts list <task-id>
```

`moat tasks logs` streams the sandbox container's own logs; Docker mode returns
"not available". Use `--previous` for a prior crashed container instance when
supported by the runtime. Without `--no-follow`, logs keep streaming. A Script's
output is not in `moat tasks logs`: `moat tasks scripts run <name>` starts it or
attaches to it and returns a terminal id, and its output is read in that
terminal in the T3 UI.

`moat tasks workspace-repo <task-id>` prints a block of `export` lines that point
the `restic` CLI at the task's own stored workspace. Platform admins only, and
only where the deployment writes the `restic` format. Read-only inspection:
writing to that repository races the sandbox.
