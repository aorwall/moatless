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
kind, description, and scope as supported by each command. A Workspace owns
image, setup commands, scripts, environment, resources, language servers, and
sandbox identity, and is edited only in git: `moat workspaces` lists, shows and
deletes legacy manual rows, and edits nothing.

Every repository has a `default` Workspace. To change it, or to add another,
edit `.moatless/workspaces.json` in the owning repository: an entry with
`"key": "default"` overrides the default and needs neither `name` nor `repos`.
An entry also takes `icon`, `defaultModel`, `defaultEffort`, and
`"skills": {"enable": [...], "disable": [...]}` naming `<plugin>` or
`<plugin>:<skill>`, which replaces the Workspace's activation records on every
sync (deleting the key keeps the last ones, so commit `{}` to clear).
`sandboxServiceAccount`, `networkProfile` and `nodeSelector` are kept only when
the deployment's `workspaceSync.identityGrants` grants that repository and key
the same value; an ungranted value is dropped with a sync warning. The sync runs
when a GitHub default branch moves; `moat repos sync-config` runs it at once.

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
