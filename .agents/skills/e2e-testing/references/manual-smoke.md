# Manual smoke verification

Use this after installation or for a narrowly scoped runtime check. Select an
existing workspace with a known Script. Inspect current CLI help and
authenticate first; do not change cluster-wide auth settings for a smoke task.

```bash
moat workspaces list --json
moat tasks create --workspace <workspace-id> --agent-profile claude-code \
  --headless false --name "Manual preview smoke"
moat tasks status <task-id> --json
moat tasks diagnose <task-id>
```

Creation provisions the sandbox. Without `--message`, no initial agent turn runs.
To verify agent execution, include an initial message requesting a small observable
change in the fixture application. Read `moat messages list <task-id> --full`,
agent state, and actual output. Task status stays Open after agent completion;
never poll for it to leave Open as a completion check.

`moat tasks status` no longer lists servers or preview URLs. List the workspace's
Scripts and start one with `moat tasks scripts list` / `moat tasks scripts run
<name>`; its output is in its terminal in the T3 UI, not `moat tasks logs`. A
Script with a `port` earns a preview tab in the T3 UI (Kubernetes only — Docker
does not publish a Script's port, so there is no preview URL to request there).
Verify the expected page or response through that tab, not just TCP readiness.

Close the disposable task with `moat tasks close <task-id>` when the check ends.
Report which of provisioning, agent behavior, and preview HTTP behavior passed.
For repeatable coverage use the suite's fixtures and cleanup instead of expanding
this manual recipe. For fleet diagnostics use
[sandbox troubleshooting](../../sandbox-troubleshooting/SKILL.md).
