---
name: handoff
description: Compact the current conversation into a new Moatless task for another agent to pick up.
argument-hint: "What will the next session be used for?"
disable-model-invocation: true
---

Create a new Moatless task that contains the handoff. Do not just write a handoff document.

The spawn message must be fully self-contained. The next agent runs in a fresh sandbox and cannot see your uncommitted scratch files, terminal history, or private reasoning. Inline the information it needs:

- Current objective and what the next task should do.
- Important decisions and constraints.
- Current repo state, branch, open PR/issue URLs, and command outputs that matter.
- Files changed or files the next agent should inspect.
- Verification already run and remaining checks.
- Blockers, risks, and exact questions still needing an answer.
- Suggested skills the next agent should invoke.

Do not duplicate long content already captured in committed or durable artifacts such as specs, plans, decision notes, issues, commits, or diffs. Reference those artifacts by repo-relative path or URL, and attach the most relevant repo files as task contexts with `--context-file`.

Redact sensitive information, such as API keys, passwords, tokens, private keys, or personally identifiable information.

If the user passed arguments, treat them as a description of what the next session will focus on and tailor the handoff message accordingly.

## Create the Task

Pick the child task's workspace before spawning. Prefer a named workspace over a bare repo because workspaces carry run config.

```bash
moat workspaces list --json | jq '.[] | {id, name, source, dockerAccess}'
moat repos list --json | jq '.[] | {id, name, remoteUrl}'   # fallback only
```

Default to the sandbox-only workspace for ordinary coding handoffs. Use a preview workspace only when the next task needs a running app for visual verification. Use a Docker-enabled workspace only when the next task needs Docker or compose.

Create the child task with `moat tasks create`. Use `--parent "$TASK_ID"` when `TASK_ID` is available, and attach relevant repo files with repeated `--context-file` flags. If the current CLI does not list `--workspace` in `moat tasks create --help`, fall back to `--repo <repo-id-or-name>`.

Choose the agent harness and model deliberately:

- Use `moat agents models` to inspect available agent harnesses and models when the choice is not already explicit.
- If the user named an agent harness or model in the handoff request, use it.
- If continuing the same kind of work, prefer the current task's agent harness.
- Otherwise choose the harness that best fits the next task from the CLI-reported options.
- Set `--model` only when a specific model is needed; omit it to use the platform default for the chosen harness.

```bash
HANDOFF_MESSAGE=$(cat <<'EOF'
# Handoff

Current objective:
...

Next task:
...

Important context:
...

Suggested skills:
...

Relevant files:
- path/to/file.ext
EOF
)

AGENT_TYPE="<agent-id>"
MODEL="" # Optional.

CREATE_ARGS=(
  --workspace "<workspace-id-or-name>"
  --name "Handoff: <short focus>"
  --agent-type "$AGENT_TYPE"
  --message "$HANDOFF_MESSAGE"
  --context-file path/to/file.ext
)

if [ -n "${TASK_ID:-}" ]; then
  CREATE_ARGS+=(--parent "$TASK_ID")
fi

if [ -n "${MODEL:-}" ]; then
  CREATE_ARGS+=(--model "$MODEL")
fi

CHILD_ID=$(moat tasks create "${CREATE_ARGS[@]}" --json | jq -r '.taskId')

echo "Created handoff task: $CHILD_ID"
```

If `TASK_ID` is not set, omit `--parent`.

Report the new task id to the user and briefly state what focus/context was handed off.
