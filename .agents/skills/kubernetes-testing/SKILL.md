---
name: kubernetes-testing
description: Operate and debug a running Moatless Vibe Kubernetes cluster — pods, logs, metrics, event delivery. Use when monitoring worker pods, debugging worker/sandbox issues, checking logs or metrics, or creating test tasks by hand against a cluster. For the automated cluster test suite, see the e2e-testing skill. Triggers on: K8s testing, kubernetes mode, worker pods, vibe task in kubernetes, pod logs, debug k8s, metrics, event delivery
---

# Kubernetes Testing

Operating and debugging a running Moatless Vibe cluster — pods, logs, metrics,
event delivery. The automated black-box suite that runs against a cluster lives
in the `e2e-testing` skill (**Cluster target**); come here when a run goes red or
you need to poke the cluster by hand.

## Targets

| Target | Namespace | Ingress | kube-context |
|--------|-----------|---------|--------------|
| local-k8s | `moatless` | `http://moatless.localhost` (Traefik) | `docker-desktop` |
| remote | `vibe` | `https://moatless.dev` | cluster context |

## Quick Reference

### Environment Setup

```bash
# API key = the cluster's TEST_API_KEY (mvk_…); the backend mints `test-api-user`.
export API_KEY="$TEST_API_KEY"
export BASE_URL="https://moatless.dev"    # local: http://moatless.localhost
export NAMESPACE="vibe"                    # local: moatless
```

**Test API Key**: On startup, if `TEST_API_KEY` is set, the backend
auto-creates a dedicated `test-api-user` and attaches the API key.
No OAuth login required.

**Setup steps:**
1. Generate key: `echo "mvk_$(openssl rand -hex 32)"`
2. Add `TEST_API_KEY` to `vibe-secrets`
3. Restart backend — test user and key created on startup (idempotent)

**Verify:**
```bash
curl -s -D - -o /dev/null -H "Authorization: Bearer $API_KEY" \
  "$BASE_URL/api/v1/auth/verify" 2>&1 | grep -i "x-user"
# Expected: x-user-role: admin
```

### Architecture

| Component | Description |
|-----------|-------------|
| Main App | `vibe-moatless-vibe-*` - API, orchestration |
| Workers | `moatless-moatless-vibe-{taskId}-*` - Claude Code execution |
| Database | PostgreSQL for persistent state |

## Vibe Task Creation (K8s Mode)

K8s clones a repo by its **remote git URL** — a vibe task references a template
repo (git URL), which the worker pod clones. No local repos, no per-run GitHub
forking. The env-agnostic suite uses the public `hello-world-ui` repo for this.

For automated creation, run the cluster suite (`e2e-testing` skill). To create
one by hand:

```bash
# List templates
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/repositories" | \
  jq '.repositories[] | select(.kind == "template") | {id, name}'

# Create a vibe task from a template repo
TEMPLATE_ID="your-template-uuid"
curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"agentType\":\"claude-code\",\"templateId\":\"$TEMPLATE_ID\",\"message\":\"Create hello world\",\"start\":true}" \
  "$BASE_URL/api/v1/tasks/vibe"
```

If the template repo is private, the backend needs a GitHub token to clone it —
the `GITHUB_PAT` env on the backend, or a user provider token via
`/api/v1/auth/provider-tokens`. A public template (the suite's default) needs none.

### Monitor Task

```bash
# Task status
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID" | \
  jq '{status, numTurns}'

# Workspace status
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/workspace-status"
```

## Quick Smoke Test

A standard workflow to verify the full K8s pipeline is working:

### Step 1: Create Task
```bash
# Get template ID
TEMPLATE_ID=$(curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/repositories" | \
  jq -r '.repositories[] | select(.kind == "template") | .id' | head -1)

# Create task with specific request
TASK_ID=$(curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"agentType\":\"claude-code\",\"templateId\":\"$TEMPLATE_ID\",\"message\":\"Create a hello world page with a purple heading\",\"start\":true}" \
  "$BASE_URL/api/v1/tasks/vibe" | jq -r '.taskId')
echo "Task: $TASK_ID"
```

### Step 2: Wait for Completion
```bash
# Poll until numTurns > 0
while true; do
  STATUS=$(curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID" | jq -r '.numTurns')
  echo "Turns: $STATUS"
  [ "$STATUS" != "0" ] && break
  sleep 10
done
```

### Step 3: Send Follow-up Message
```bash
curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"message":"Change the heading color from purple to orange"}' \
  "$BASE_URL/api/v1/tasks/$TASK_ID/messages"
```

### Step 4: Verify Preview Server
```bash
# Get preview URL
PREVIEW_URL=$(curl -s -H "Authorization: Bearer $API_KEY" \
  "$BASE_URL/api/v1/tasks/$TASK_ID/server/status" | jq -r '.url')
echo "Preview: $PREVIEW_URL"

# Verify it responds (401 expected without auth, 200 with auth)
curl -s -o /dev/null -w "%{http_code}" -H "Authorization: Bearer $API_KEY" "$PREVIEW_URL"
```

### Step 5: Check for Errors
```bash
# Verify no "Agent Error" in messages
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/messages" | \
  grep -o "Agent Error" || echo "✓ No Agent Errors found"
```

## Pod Management

### Check Pods

```bash
# All pods
kubectl get pods -n $NAMESPACE

# Worker for specific task
kubectl get pods -n $NAMESPACE -l moatless.io/task-id=$TASK_ID
```

### Check Logs

```bash
# Backend logs
kubectl logs deployment/moatless-vibe-backend -n $NAMESPACE --tail=100

# Worker/sandbox logs
WORKER_POD=$(kubectl get pods -n $NAMESPACE -l moatless.io/task-id=$TASK_ID -o jsonpath='{.items[0].metadata.name}')
kubectl logs $WORKER_POD -n $NAMESPACE -c sandbox

# Filter for errors (tracing JSON format)
kubectl logs deployment/moatless-vibe-backend -n $NAMESPACE --tail=200 | grep -i '"level":"ERROR"'
```

## Metrics Monitoring

Access Prometheus metrics at `/metrics` (requires `METRICS_ENABLED=true`).

```bash
kubectl port-forward deployment/moatless-vibe-backend -n $NAMESPACE 9090:9090
curl -s localhost:9090/metrics | grep moatless_
```

### Key Metrics for Testing

| Category | Key Metrics | What to Check |
|----------|-------------|---------------|
| **Events** | `event_delivery_total{mode,status}` | NOTIFY vs poll success rates |
| **Agents** | `active_agents`, `agent_execution_total` | Running agents, success/error |
| **Tasks** | `task_status_transitions_total` | Task lifecycle progression |
| **K8s** | `kubernetes_pod_total`, `pod_readiness_duration` | Pod startup <60s, no failures |
| **Database** | `database_operation_total{status}` | Error rate should be 0 |
| **Errors** | `error_total{service,severity}` | Any errors during testing |

### Quick Health Check

```bash
# Active resources
curl -s localhost:9090/metrics | grep -E "active_(agents|workspaces)"

# Event delivery (NOTIFY should dominate)
curl -s localhost:9090/metrics | grep event_delivery_total

# Any errors
curl -s localhost:9090/metrics | grep error_total

# Database health
curl -s localhost:9090/metrics | grep 'database_operation_total.*error'
```

See `references/metrics.md` for complete metrics reference with all labels and testing scenarios.

## Common Issues

### "Local repositories not supported in Kubernetes mode"

**Cause**: Task references a local repo. K8s clones by remote git URL only.

**Fix**: Point the task at a repo with a remote git URL (the suite uses the public `hello-world-ui`).

### "No execution target registered"

**Cause**: Main app restarted, in-memory target map cleared.

**Resolution**: Send a new message to re-register the target, or restart the worker pod.

### ImagePullBackOff

**Cause**: Wrong platform or image doesn't exist.

**Fix**: Rebuild with correct platform:
```bash
docker buildx build --platform linux/amd64 -t aorwall/moatless-frontend:latest -f docker/Dockerfile.app --push .
```

## Automated tests

The env-agnostic cluster suite (`tests/cluster`) is the automated path — smoke,
lifecycle, and agent rungs against a live cluster. See the `e2e-testing` skill's
**Cluster target** section, or `references/integration-tests.md`.

## Deployment

```bash
# Restart main app
kubectl rollout restart deployment/moatless-vibe-backend -n $NAMESPACE

# Delete worker pod
kubectl delete pod $WORKER_POD -n $NAMESPACE
```

## Resources

- `references/api_reference.md` - Complete API workflows for task creation and verification
- `references/integration-tests.md` - Running the env-agnostic cluster suite against k8s
- `references/debugging.md` - Detailed debugging commands
- `references/metrics.md` - Complete Prometheus metrics reference with labels and testing scenarios
