# Moatless Admin

Operational skills for running the Moatless platform itself — sizing sandboxes,
reading the cluster's resource metrics, and applying the result.

These are packaged as a plugin rather than left in the repository-level
`.agents/skills` because operating the platform happens from wherever the
operator is: another machine, a sandbox, a cluster shell. A plugin travels;
`.agents/skills` does not.

## Skills

| Skill | Use it when |
|---|---|
| `workspace-resource-tuning` | Deciding which workspaces need more memory, why the sandbox pool is running more nodes than it should, or how to reclaim reserved-but-unused CPU and memory. |

## Scope

Proactive operation and tuning. For the reactive case — a task is stuck, a
sandbox was OOMKilled or evicted, a pod won't start — use the repository's
`sandbox-troubleshooting` skill instead.
