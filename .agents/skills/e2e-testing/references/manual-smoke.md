# Manual smoke verification

Use this after installation or for a narrowly scoped runtime check. Select an
existing workspace with a known preview configuration. Inspect current CLI help
and authenticate first; do not change cluster-wide auth settings for a smoke task.

```bash
moat workspaces list --json
moat tasks create --workspace <workspace-id> --agent-type claude-code \
  --headless false --name "Manual preview smoke"
moat tasks status <task-id> --json
moat tasks diagnose <task-id>
```

Creation provisions the sandbox. Without `--message`, no initial agent turn runs.
To verify agent execution, include an initial message requesting a small observable
change in the fixture application. Read `moat messages list <task-id> --full`,
agent state, and actual output. Task status stays Open after agent completion;
never poll for it to leave Open as a completion check.

Read the returned server names and URLs from status. Inspect bounded logs with
`moat tasks logs <task-id> --server <name> --no-follow --tail 500`, then request the
returned URL using the installation's authentication. Verify the expected page or
response, not just TCP readiness. Preview servers require `--headless false`.

Close the disposable task with `moat tasks close <task-id>` when the check ends.
Report which of provisioning, agent behavior, and preview HTTP behavior passed.
For repeatable coverage use the suite's fixtures and cleanup instead of expanding
this manual recipe. For fleet diagnostics use
[sandbox troubleshooting](../../sandbox-troubleshooting/SKILL.md).
