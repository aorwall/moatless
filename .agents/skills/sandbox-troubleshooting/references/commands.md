# Diagnostic commands

Use the task ID for CLI calls and the returned sandbox ID for runtime lookup.

```bash
moat tasks get <task-id> --json | jq '{taskId, sandboxId, workspaceId, status}'
moat tasks diagnose <task-id>
moat tasks status <task-id> --json
moat events list <task-id> --errors --json
moat messages list <task-id> --full
moat messages list <task-id> --limit 200
moat messages list <task-id> --before-id <message-id>
moat tasks logs <task-id> --no-follow --tail 500
moat tasks logs <task-id> --previous --no-follow --tail 500
```

`moat tasks logs` streams the sandbox container's own logs (Docker mode answers
"not available"). CLI logs follow by default; use `--no-follow` for a
terminating tool call. Increase `--tail` when startup output fell out of the
window. `--previous` reads the prior container instance where the runtime
supports it, not an arbitrary older pod.

Correlate events, logs, and the transcript by timestamp. An idle agent may be
waiting for user input; an Open task does not imply an active turn. Do not use
cost or lifecycle status alone to conclude that work completed.
