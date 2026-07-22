# Kubernetes Testing Workflows

Workflows for creating tasks and verifying agent/preview execution in K8s mode.
Routes below are the real backend surface — task/repository APIs live under
`/api/v1/…`; sandbox (workspace/server) APIs are backend-proxied under
`/api/sandbox/v1/tasks/{task_id}/…`.

## Environment Setup

```bash
# Test API key — auto-creates test user on backend startup (see SKILL.md for setup)
export API_KEY="$TEST_API_KEY"
export BASE_URL="https://moatless.dev"
export NAMESPACE="vibe"
```

> `TEST_API_KEY` must be set in `vibe-secrets`. The backend auto-creates
> a `test-api-user` on startup. No OAuth required.

---

## Create + Start a Task

There is one creation path: `POST /api/v1/tasks` with a `repositoryId` (point it
at a `remote` repo or a `template` repo — same call). Creation does not run the
agent; start it with `POST /api/v1/tasks/{id}/start`. There is no `/tasks/vibe`
route, no `templateId` field, and no `start` body field.

### Step 1: Get a Repository ID

```bash
# The endpoint returns a bare array of repositories
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/repositories" | \
  jq '.[] | select(.kind == "template") | {id, name}'   # or .kind == "remote"
```

### Step 2: Create the Task

```bash
REPO_ID="your-repository-uuid"

RESPONSE=$(curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "{
    \"agentType\": \"claude-code\",
    \"repositoryId\": \"$REPO_ID\",
    \"message\": \"Create a hello world page\"
  }" \
  "$BASE_URL/api/v1/tasks")

TASK_ID=$(echo "$RESPONSE" | jq -r '.taskId')
echo "Task ID: $TASK_ID"
```

**Request body (`CreateTaskRequest`):**
| Field | Required | Description |
|-------|----------|-------------|
| repositoryId | one of repositoryId / workspaceId | UUID of the repository (template or remote) |
| workspaceId | one of repositoryId / workspaceId | Explicit workspace (wins over repositoryId) |
| agentType | No | Agent type (e.g. `claude-code`) |
| message | No | Initial task message |
| model | No | Model override |
| contexts | No | Additional context items |

Response (`CreateTaskResponse`): `{ taskId, status, task }`.

### Step 3: Start the Agent

```bash
curl -s -X POST -H "Authorization: Bearer $API_KEY" \
  "$BASE_URL/api/v1/tasks/$TASK_ID/start"
```

### Step 4: Verify the Worker Pod (K8s)

```bash
sleep 10
kubectl get pods -n $NAMESPACE -l moatless.io/task-id=$TASK_ID
WORKER_POD=$(kubectl get pods -n $NAMESPACE -l moatless.io/task-id=$TASK_ID -o jsonpath='{.items[0].metadata.name}')
```

Worker Deployment/Service/pods are named `moatless-ws-{task_id}`.

### Step 5: Monitor Task Status

```bash
# Task-level status: open | closed | error
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID" | \
  jq '{status, title}'

# Sandbox / workspace status (backend-proxied sandbox API)
curl -s -H "Authorization: Bearer $API_KEY" \
  "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/status"

# Live status (lighter-weight poll)
curl -s -H "Authorization: Bearer $API_KEY" \
  "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/live-status"
```

### Step 6: Send a Follow-up Message

```bash
curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"message": "Now update the heading color to orange"}' \
  "$BASE_URL/api/v1/tasks/$TASK_ID/messages"
```

---

## Verify Agent Execution

### Via kubectl (K8s)

```bash
WORKER_POD=$(kubectl get pods -n $NAMESPACE -l moatless.io/task-id=$TASK_ID -o jsonpath='{.items[0].metadata.name}')

kubectl get pod $WORKER_POD -n $NAMESPACE
kubectl logs $WORKER_POD -n $NAMESPACE -c sandbox --tail=100

# Main app logs for this task
kubectl logs deployment/moatless-vibe-backend -n $NAMESPACE --tail=200 | grep $TASK_ID
```

The sandbox container's own HTTP server only exposes `/health` and `/livez` —
agent/server state comes from the backend-proxied sandbox API above, not from
container-local endpoints.

---

## Verify Preview Server

### Server status via API

```bash
curl -s -H "Authorization: Bearer $API_KEY" \
  "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/server/status" | jq '.servers'
```

Response is `{ servers: [ ... ] }`; each server snapshot includes `name`,
`status`, `url` (external proxy URL), `error`, `detail`, `default`, and
`capabilities`. The preview URL is the default server's `url`:

```bash
PREVIEW_URL=$(curl -s -H "Authorization: Bearer $API_KEY" \
  "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/server/status" | \
  jq -r '.servers[] | select(.default) | .url')
curl -s -o /dev/null -w "%{http_code}\n" -H "Authorization: Bearer $API_KEY" "$PREVIEW_URL"
```

### Restart / stop / redeploy the sandbox

```bash
curl -s -X POST -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/restart"
curl -s -X POST -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/stop"
curl -s -X POST -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/redeploy"
```

### Stream server logs (SSE)

```bash
curl -N -H "Authorization: Bearer $API_KEY" \
  "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/servers/preview/logs"
# add ?previous=true for a crashed/restarted container
# or: moat tasks logs $TASK_ID --server preview
```

### Direct preview check via kubectl

```bash
WORKER_POD=$(kubectl get pods -n $NAMESPACE -l moatless.io/task-id=$TASK_ID -o jsonpath='{.items[0].metadata.name}')
kubectl exec $WORKER_POD -n $NAMESPACE -c sandbox -- curl -s localhost:3000 | head -20
```

---

## Complete E2E Test Script

```bash
#!/bin/bash
set -e

API_KEY="${API_KEY:-mvk_your_api_key}"
BASE_URL="${BASE_URL:-https://moatless.dev}"
NAMESPACE="${NAMESPACE:-vibe}"

echo "=== 1. Create + start task ==="
TEMPLATE_ID=$(curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/repositories" | \
  jq -r '.[] | select(.kind == "template") | .id' | head -1)
TASK_ID=$(curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"agentType\":\"claude-code\",\"repositoryId\":\"$TEMPLATE_ID\",\"message\":\"Create hello world\"}" \
  "$BASE_URL/api/v1/tasks" | jq -r '.taskId')
curl -s -X POST -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/start" >/dev/null
echo "Task: $TASK_ID"

echo "=== 2. Wait for worker pod ==="
sleep 15
kubectl get pods -n $NAMESPACE -l moatless.io/task-id=$TASK_ID

echo "=== 3. Wait for completion (status leaves 'open') ==="
for i in {1..30}; do
  STATUS=$(curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID" | jq -r '.status')
  echo "[$i] Status: $STATUS"
  [[ "$STATUS" != "open" ]] && break
  sleep 5
done

echo "=== 4. Check preview server ==="
curl -s -H "Authorization: Bearer $API_KEY" \
  "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/server/status" | jq '.servers'

echo "=== Done ==="
```
