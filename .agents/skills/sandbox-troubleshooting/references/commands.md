# Diagnostic command reference

All examples assume `NS=moatless` (the sandbox namespace) and `TASK_ID` set to
the full task UUID (see SKILL.md Step 0 to resolve a shortId).

## moat CLI — start here (works from anywhere, no cluster creds needed)

```bash
# One-shot aggregate: status + containerDiagnostics + runtimeEvents + recent errors
moat tasks diagnose $TASK_ID [--limit 50]

# Current environment / agent / per-server status
moat tasks status $TASK_ID
# Plain status .containerDiagnostics carries only: name, state, restartCount.
# `moat tasks diagnose` enriches each entry with cpuMillicores + memoryMb, and
# terminated containers add exitCode/lastExitCode/terminatedReason/waitingReason:
moat tasks status $TASK_ID --json | jq '.containerDiagnostics'
moat tasks diagnose $TASK_ID --json | jq '.status.containerDiagnostics'   # + cpu/mem, exit, OOM reason

# Durable event history (the source of truth once a pod is gone)
moat events list $TASK_ID --types sandbox,server --limit 400 --json
moat events list $TASK_ID --errors                 # only isError:true
moat events list $TASK_ID --follow                 # live SSE tail

# Filter events to a time window (events are UTC!)
# NOTE: event payload keys are camelCase (lifecycleEvent, serverName, eventType) —
#       snake_case (lifecycle_event) silently returns empty. VALUES stay snake_case
#       (pod_scheduled, container_oom_killed, idle_reaped, ...).
moat events list $TASK_ID --limit 400 --json \
  | jq -r '.[] | "\(.timestamp) [\(.type)/\(.subType)] err=\(.isError) \(.payload.lifecycleEvent // "")"' \
  | tail -80

# Live logs for one container (server: sandbox | preview | files | <name>-server)
# NOTE: `moat tasks logs` STREAMS/follows (blocking, no --tail/--follow flag) — wrap
#       in `timeout 15 …` for a snapshot, or use `kubectl logs --tail=N` for a bounded read.
moat tasks logs $TASK_ID --server sandbox
moat tasks logs $TASK_ID --server preview --previous   # crashed/restarted instance
# NOTE: --previous returns 503 once the pod is fully gone; use the event stream instead.

# Run transcript — what the agent actually did (stalls, tool errors, OOM context)
moat messages list $TASK_ID --full                 # full transcript (roles, timestamps, tool markers)
moat messages list $TASK_ID                         # compact preview table (newest 50)
moat messages list $TASK_ID --limit 200             # bigger page (max 200)
moat messages list $TASK_ID --before-id <id>        # page further back (cursor from prev page)
# JSON shape: { messages: [{ role, timestamp, content, messageType, turnNumber, usage,
#              model, agentMode, toolResults, sessionId, messageId, ... }],
#              totalCount, hasMore, nextBeforeId }  — page with --before-id $(… .nextBeforeId)
# `content` is an ARRAY of typed blocks (thinking / text / tool_use), not a string —
# extract text with: .content[]? | select(.type=="text") | .text
moat messages list $TASK_ID --json \
  | jq -r '.messages[] | "\(.timestamp) \(.role): \([.content[]? | select(.type=="text") | .text] | join(" "))"'

# Files in the workspace
moat tasks files $TASK_ID tree
moat tasks files $TASK_ID read --path AGENTS.md

# Lifecycle actions
moat tasks restart $TASK_ID      # recreate pod, keep PVC
moat tasks redeploy $TASK_ID     # fresh repo config, keep PVC
moat tasks start $TASK_ID        # resume a stopped/idle-reaped task
```

## kubectl — pod JSON (works from inside a sandbox; events/top may be Forbidden)

```bash
# Find the pod (Running one, avoids terminating pods during rollout)
POD=$(kubectl get pod -n $NS -l moatless.io/task-id=$TASK_ID \
  --field-selector=status.phase=Running -o jsonpath='{.items[0].metadata.name}')

# Pod summary
kubectl get pod -n $NS -l moatless.io/task-id=$TASK_ID -o wide   # STATUS, RESTARTS, NODE

# OOM / crash detection (substitute for Forbidden `kubectl get events`)
kubectl get pod $POD -n $NS -o jsonpath='{range .status.containerStatuses[*]}{.name}: restarts={.restartCount} waiting={.state.waiting.reason} lastReason={.lastState.terminated.reason} exit={.lastState.terminated.exitCode}{"\n"}{end}'

# Eviction reason (substitute for `kubectl describe pod` Events)
kubectl get pod $POD -n $NS -o jsonpath='{.status.reason}{" "}{.status.message}{"\n"}'

# Container resource limits (for OOM analysis)
kubectl get deploy moatless-ws-$TASK_ID -n $NS -o json \
  | jq '.spec.template.spec.containers[] | {name, resources}'

# Logs (current + previous crashed instance)
kubectl logs $POD -n $NS -c sandbox --tail=200
kubectl logs $POD -n $NS -c sandbox --previous --tail=200

# K8s events + live resource usage (granted to the sandbox SA on chart >= 0.1.1)
kubectl get events -n $NS --field-selector involvedObject.name=$POD --sort-by=.lastTimestamp
kubectl top pod $POD -n $NS --containers
# If these return Forbidden you're on an older release — use `moat tasks diagnose` instead.

# Node checks — is a "scary" restart just a spot reclaim?
kubectl get node <node-name>                 # NotFound => node gone (reclaimed)
kubectl get nodes -L karpenter.sh/capacity-type,node.kubernetes.io/instance-type
kubectl top nodes

# Scale (patch is allowed for the sandbox SA; delete/create are not)
kubectl scale deploy moatless-ws-$TASK_ID -n $NS --replicas=0   # stop
kubectl scale deploy moatless-ws-$TASK_ID -n $NS --replicas=1   # start

# Shell into a container — FORBIDDEN for the moatless-sandbox SA (can't create
# pods/exec); needs an admin kubeconfig. From inside a sandbox use `moat tasks
# files`/`moat tasks logs` instead of exec.
kubectl exec -it $POD -n $NS -c sandbox -- bash   # admin-only
```

## Backend logs & metrics (needs backend/cluster access, not the sandbox SA)

```bash
# Pod watcher fatal failures carry the last 50 sandbox log lines for root cause
kubectl logs deploy/moatless-vibe-backend -n $NS | grep 'Sandbox pod failed irrecoverably'
kubectl logs deploy/moatless-vibe-backend -n $NS | grep -E 'lifecycle_event|idle_reaped|re-provisioning'

# Prometheus metrics
kubectl port-forward deploy/moatless-vibe-backend -n $NS 9090:9090
curl -s localhost:9090/metrics | grep -E 'sandbox_container_terminations_total|task_reconcile_remediations_total|moatless_sandbox_startup_seconds'
```
