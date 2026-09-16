# Narrow the failure

| Symptom | Evidence to collect | Next action |
|---|---|---|
| Sandbox will not start | Diagnose, pod conditions, scheduling events, init logs, image pull error | Fix the specific image, mount, quota, credential, or scheduling constraint. |
| Restarted or OOMKilled | Previous container logs, termination reason, events, resource limits, transcript at that time | Reduce concurrent work or change workspace resource limits; keep enough evidence to distinguish memory from disk pressure. |
| Evicted | Pod reason/message, node events, workspace disk use | Relieve the named pressure. A larger memory limit does not repair exhausted ephemeral storage. |
| Pod vanished, or its node went NotReady | The pod's `spec.nodeName`, that node's `karpenter.sh/capacity-type` label and taints, its NodeClaim, node events in `default` | Follow [node disruption](node-disruption.md). A spot reclaim or a consolidation is not repaired by changing the workspace's resources. |
| Agent unresponsive | Agent state, last message/tool, permission request, sandbox connection and logs | Resolve the blocked input/tool or transport fault; do not infer a hang from Open task status. |
| Server failed | Server status and bounded server logs | Follow the preview skill's live override workflow. |
| Stops after inactivity | Lifecycle events and configured idle timeout | Confirm idle policy before treating an intentional stop as a crash. |
| CLI and pod disagree | Timestamps, pod UID, all matching sandbox pods | Trace watcher and status resolver in the code map; do not assume the newest Running pod is ready. |

Resource changes belong to `moat workspaces update <workspace-id> --resources-json`.
Read the existing config and preserve all fields when replacing JSON. For git-owned
configuration, edit its source or explicitly take manual ownership. Existing
containers require reprovisioning to receive changed container limits.
