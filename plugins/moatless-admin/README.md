# Moatless Admin

Operational skills for running the Moatless platform itself — sizing sandboxes,
reading the cluster's resource metrics and logs, reporting what they say, and
administering the deployment's configuration.

These are packaged as a plugin rather than left in the repository-level
`.agents/skills` because operating the platform happens from wherever the
operator is: another machine, a sandbox, a cluster shell. A plugin travels;
`.agents/skills` does not.

## Skills

| Skill | Use it when |
|---|---|
| `workspace-resource-tuning` | Deciding which workspaces need more memory, why the sandbox pool is running more nodes than it should, or how to reclaim reserved-but-unused CPU and memory. |
| `daily-health-report` | Reporting one day of the platform — tasks, cost, Kubernetes warning events, logs and feedback — as an HTML page a person reads. The `daily-health-check` Loop runs it on a schedule. |
| `moatless-admin` | Configuring the deployment as an admin: users, workspaces, loops, plugins, secrets, apps, and the git and Helm config behind them. Small named changes are made directly; larger ones go through an admin page where a person reviews the state and approves a plan. |

## Scope

Proactive operation and tuning. For the reactive case — a task is stuck, a
sandbox was OOMKilled or evicted, a pod won't start — use the repository's
`sandbox-troubleshooting` skill instead.
