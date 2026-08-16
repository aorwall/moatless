---
name: kubernetes-testing
description: Operate and debug a running Moatless Vibe Kubernetes cluster — pods, logs, metrics, event delivery. Use when monitoring worker pods, debugging worker/sandbox issues, checking logs or metrics, or creating test tasks by hand against a cluster. Triggers on: K8s testing, kubernetes mode, worker pods, vibe task in kubernetes, pod logs, debug k8s, metrics, event delivery
---

# Kubernetes Testing

Operating and debugging a running Moatless Vibe cluster by hand — pods, logs,
metrics, event delivery. Come here to create a task manually and watch the
pipeline, or when something in the cluster needs poking.

## Targets

| Target | Namespace | Ingress | kube-context |
|--------|-----------|---------|--------------|
| local-k8s | `moatless` | `http://moatless.localhost` (Traefik) | `docker-desktop` |
| remote | `<namespace>` | deployment URL | `<cluster-context>` |

## Quick Reference

### Environment Setup

```bash
# API key = the cluster's TEST_API_KEY (mvk_…); the backend mints `test-api-user`.
export API_KEY="$TEST_API_KEY"
export BASE_URL="https://moatless.dev"    # local: http://moatless.localhost
export NAMESPACE="<namespace>"                 # e.g. moatless
export KUBE_CONTEXT="<target-context>"         # use the named target; never rely on current-context
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

| Component | Pods | Description |
|-----------|------|-------------|
| Main App | `moatless-vibe-backend-*` | API, orchestration |
| Workers | `moatless-ws-{taskId}-*` | per-task Deployment/Service running Claude Code |
| Database | PostgreSQL | persistent state |

Worker resources are named deterministically `moatless-ws-{task_id}` and carry
the label `moatless.io/task-id={task_id}` (see `sandbox/k8s/naming.rs`).

## Vibe Task Creation (K8s Mode)

K8s clones a repo by its **remote git URL** — a task references a template repo
(by its repository id), which the worker pod clones. No local repos, no per-run
GitHub forking. Use the public `hello-world-ui` template for smoke tests.

Task creation is `POST /api/v1/tasks` with a `repositoryId` (there is no
`/tasks/vibe` route and no `templateId` field). Creating does not auto-run —
start it with `POST /api/v1/tasks/{id}/start`.

```bash
# List template repos (the endpoint returns a bare array)
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/repositories" | \
  jq '.[] | select(.kind == "template") | {id, name}'

# Create a task against a template repo, then start it
TEMPLATE_ID="your-template-uuid"
TASK_ID=$(curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"agentType\":\"claude-code\",\"repositoryId\":\"$TEMPLATE_ID\",\"message\":\"Create hello world\"}" \
  "$BASE_URL/api/v1/tasks" | jq -r '.taskId')

curl -s -X POST -H "Authorization: Bearer $API_KEY" \
  "$BASE_URL/api/v1/tasks/$TASK_ID/start" >/dev/null
echo "Task: $TASK_ID"
```

If the template repo is private, the backend needs a GitHub token to clone it —
the `GITHUB_TOKEN` env on the backend, or a user-provided GitHub provider token.
A public template needs none.

### Monitor Task

```bash
# Task status (open | closed | error)
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID" | \
  jq '{status, title}'

# Sandbox / workspace status (backend-proxied sandbox API)
curl -s -H "Authorization: Bearer $API_KEY" \
  "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/status"
```

## Quick Smoke Test

A standard workflow to verify the full K8s pipeline is working:

### Step 1: Create + start Task
```bash
TEMPLATE_ID=$(curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/repositories" | \
  jq -r '.[] | select(.kind == "template") | .id' | head -1)

TASK_ID=$(curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"agentType\":\"claude-code\",\"repositoryId\":\"$TEMPLATE_ID\",\"message\":\"Create a hello world page with a purple heading\"}" \
  "$BASE_URL/api/v1/tasks" | jq -r '.taskId')
curl -s -X POST -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/start" >/dev/null
echo "Task: $TASK_ID"
```

### Step 2: Wait for Completion
```bash
# Poll until the task leaves 'open'
while true; do
  STATUS=$(curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID" | jq -r '.status')
  echo "Status: $STATUS"
  [ "$STATUS" != "open" ] && break
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
# Preview URL comes from the default server in the sandbox server status
PREVIEW_URL=$(curl -s -H "Authorization: Bearer $API_KEY" \
  "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/server/status" | \
  jq -r '.servers[] | select(.default) | .url')
echo "Preview: $PREVIEW_URL"

# Verify it responds (401 expected without auth, 200 with auth)
curl -s -o /dev/null -w "%{http_code}" -H "Authorization: Bearer $API_KEY" "$PREVIEW_URL"
```

### Step 5: Check for Errors
```bash
# Verify no "Agent Error" in messages
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/messages" | \
  grep -o "Agent Error" || echo "No Agent Errors found"
```

## Pod Management

### Check Pods

```bash
# All pods
kubectl --context "$KUBE_CONTEXT" get pods -n "$NAMESPACE"

# Worker for a specific task
kubectl --context "$KUBE_CONTEXT" get pods -n "$NAMESPACE" -l moatless.io/task-id="$TASK_ID"
```

### Check Logs

```bash
# Backend logs
kubectl --context "$KUBE_CONTEXT" logs deployment/moatless-vibe-backend -n "$NAMESPACE" --tail=100

# Worker/sandbox logs
WORKER_POD=$(kubectl --context "$KUBE_CONTEXT" get pods -n "$NAMESPACE" -l moatless.io/task-id="$TASK_ID" -o jsonpath='{.items[0].metadata.name}')
kubectl --context "$KUBE_CONTEXT" logs "$WORKER_POD" -n "$NAMESPACE" -c sandbox

# Filter for errors (tracing JSON format)
kubectl --context "$KUBE_CONTEXT" logs deployment/moatless-vibe-backend -n "$NAMESPACE" --tail=200 | grep -i '"level":"ERROR"'
```

## Metrics Monitoring

Prometheus metrics are on `/metrics` at the backend metrics port (`METRICS_PORT`,
default 9090). Always registered — there is no `METRICS_ENABLED` flag.

```bash
kubectl --context "$KUBE_CONTEXT" port-forward deployment/moatless-vibe-backend -n "$NAMESPACE" 9090:9090
curl -s localhost:9090/metrics | grep moatless_
```

### Key Metrics for Testing

| Area | Key Metrics | What to Check |
|------|-------------|---------------|
| **Agents** | `moatless_agent_runs_total`, `moatless_agent_tool_calls_total` | Run outcomes (completed/error/stopped), tool errors |
| **Cost** | `moatless_agent_cost_usd` | Per-run spend |
| **Sandbox** | `moatless_sandbox_provision_duration_seconds` | Provision time by storage_backend |
| **Workspace sync** | `moatless_workspace_sync_total` | Sync outcomes (ok/error) |
| **Events** | `events_publish_failures_total`, `events_publish_retries_total` | Redis publish health |
| **Integrations** | `moatless_adapter_*`, `moatless_binding_*`, `moatless_trigger_job_total` | Webhook/routing errors |

See `references/metrics.md` for the full list with labels. (An older metric set
like `moatless_event_delivery_total` / `moatless_active_agents` is legacy and no
longer emitted.)

## Common Issues

### "Local repositories not supported in Kubernetes mode"

**Cause**: Task references a local repo. K8s clones by remote git URL only.

**Fix**: Point the task at a repo with a remote git URL (e.g. the public `hello-world-ui`).

### "No execution target registered"

**Cause**: Main app restarted, in-memory target map cleared.

**Resolution**: Send a new message to re-register the target, or restart the worker pod.

### ImagePullBackOff

**Cause**: Wrong platform or image doesn't exist.

**Fix**: Rebuild with correct platform:
```bash
docker buildx build --platform linux/amd64 -t aorwall/moatless-frontend:latest -f docker/Dockerfile.app --push .
```

## Deployment

```bash
# Restart main app
kubectl --context "$KUBE_CONTEXT" rollout restart deployment/moatless-vibe-backend -n "$NAMESPACE"

# Delete worker pod
kubectl --context "$KUBE_CONTEXT" delete pod "$WORKER_POD" -n "$NAMESPACE"
```

## Resources

- `references/api_reference.md` - API workflows for task creation and verification
- `references/debugging.md` - Detailed debugging commands
- `references/metrics.md` - Prometheus metrics reference with labels
```
