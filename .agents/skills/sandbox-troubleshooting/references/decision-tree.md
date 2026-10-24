# Narrow the failure

| Symptom | Evidence to collect | Next action |
|---|---|---|
| Sandbox will not start | Diagnose, pod conditions, scheduling events, init logs, image pull error | Fix the specific image, mount, quota, credential, or scheduling constraint. |
| Restarted or OOMKilled | Previous container logs, termination reason, events, resource limits, transcript at that time | Reduce concurrent work or change workspace resource limits; keep enough evidence to distinguish memory from disk pressure. |
| Evicted | Pod reason/message, node events, workspace disk use | Relieve the named pressure. A larger memory limit does not repair exhausted ephemeral storage. |
| Pod vanished, or its node went NotReady | The pod's `spec.nodeName`, that node's `karpenter.sh/capacity-type` label and taints, its NodeClaim, node events in `default` | Follow [node disruption](node-disruption.md). A spot reclaim or a consolidation is not repaired by changing the workspace's resources. |
| Agent unresponsive | Agent state, last message/tool, permission request, sandbox connection and logs | Resolve the blocked input/tool or transport fault; do not infer a hang from Open task status. |
| Script failed to start or serve its port | `moat tasks scripts list <task-id>`, the script's terminal output in the T3 UI | Check the command and, under Kubernetes, whether the port was published; Docker never publishes a Script's port. |
| Stops after inactivity | Lifecycle events and configured idle timeout | Confirm idle policy before treating an intentional stop as a crash. |
| CLI and pod disagree | Timestamps, pod UID, all matching sandbox pods | Trace watcher and status resolver in the code map; do not assume the newest Running pod is ready. |

Resource changes belong to the Workspace's `resources` in its repository's
`.moatless/workspaces.json`, then `moat repos sync-config <repo>`. Existing
containers require reprovisioning to receive changed container limits.
