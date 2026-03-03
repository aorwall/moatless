# Kubernetes Testing Workflows

Complete workflows for creating tasks and verifying agent/preview execution in K8s mode.

## Environment Setup

```bash
export API_KEY="mvk_your_api_key"
export BASE_URL="https://moatless.dev"
export NAMESPACE="vibe"
```

---

## Create Regular Task Workflow

### Step 1: Get Repository ID

```bash
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/repositories" | \
  jq '.repositories[] | select(.kind == "remote") | {id, name}'
```

### Step 2: Create Task

```bash
REPO_ID="your-repository-uuid"

RESPONSE=$(curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "{
    \"agentType\": \"claude-code\",
    \"repositoryId\": \"$REPO_ID\",
    \"message\": \"Create a hello.txt file\",
    \"start\": true
  }" \
  "$BASE_URL/api/v1/tasks")

TASK_ID=$(echo $RESPONSE | jq -r '.taskId')
echo "Task ID: $TASK_ID"
```

**Request Body:**
| Field | Required | Description |
|-------|----------|-------------|
| agentType | Yes | Agent type (e.g., `claude-code`) |
| repositoryId | Yes | UUID of the repository |
| message | No | Initial task message |
| start | No | Auto-start the agent (default: false) |
| model | No | Model override |
| contexts | No | Additional context items |

### Step 3: Verify Worker Pod (K8s)

```bash
# Wait for pod creation
sleep 10

# Check worker pod exists
kubectl get pods -n $NAMESPACE -l moatless.io/task-id=$TASK_ID

# Get worker pod name
WORKER_POD=$(kubectl get pods -n $NAMESPACE -l moatless.io/task-id=$TASK_ID | awk '{print $1}')
```

### Step 4: Monitor Task Status

```bash
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID" | \
  jq '{status, iterationStatus: .iterations[-1].status, numTurns}'
```

**Iteration Status Values:**
- `created` - Iteration created, not started
- `active` - Agent running
- `completed` - Successfully finished
- `failed` - Error occurred

### Step 5: Verify Agent Execution

```bash
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/workspace-status" | \
  jq '{agentState, environmentStatus, environmentError}'
```

**Response Fields:**
| Field | Values | Description |
|-------|--------|-------------|
| agentState | `idle`, `running`, `none` | Current agent state |
| environmentStatus | `ready`, `initializing`, `error` | Environment state |
| environmentError | string | Error message if failed |

### Step 6: Send Follow-up Message

```bash
curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"message": "Now update hello.txt to say Goodbye"}' \
  "$BASE_URL/api/v1/tasks/$TASK_ID/messages"
```

---

## Create Vibe Task Workflow (K8s Mode)

In K8s mode, vibe tasks fork templates to GitHub:
```
Template → Create GitHub repo → Push code → Remote repo → K8s clones ✓
```

### Step 1: List Templates

```bash
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/repositories" | \
  jq '.repositories[] | select(.kind == "template") | {id, name}'
```

### Step 2: Create Vibe Task

```bash
TEMPLATE_ID="your-template-uuid"

RESPONSE=$(curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "{
    \"agentType\": \"claude-code\",
    \"templateId\": \"$TEMPLATE_ID\",
    \"message\": \"Create a simple hello world page\",
    \"start\": true
  }" \
  "$BASE_URL/api/v1/tasks/vibe")

TASK_ID=$(echo $RESPONSE | jq -r '.taskId')
echo "Task ID: $TASK_ID"
```

**Request Body:**
| Field | Required | Description |
|-------|----------|-------------|
| agentType | Yes | Agent type (e.g., `claude-code`) |
| templateId | Yes | UUID of the template repository |
| message | No* | Initial task message |
| contexts | No* | Context items (*one of message/contexts required) |
| start | No | Auto-start (default: true for vibe) |

### Step 3: Verify GitHub Repository Created

```bash
# Check repository is remote (not local)
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID" | \
  jq '.repositoryName'

# Verify via repositories endpoint
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/repositories" | \
  jq ".repositories[] | select(.name | contains(\"$TASK_ID\")) | {kind, url: .remote.url}"
```

**Expected:** `kind: "remote"` with GitHub URL

### Step 4-6: Same as Regular Task

Follow steps 3-6 from the regular task workflow above.

---

## Verify Agent Execution

### Via API

```bash
# Full workspace status
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/workspace-status" | jq .

# Key fields to check
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/workspace-status" | \
  jq '{
    agentState,
    environmentStatus,
    environmentType,
    capabilities,
    isRunning,
    canExecute
  }'
```

### Via kubectl (K8s)

```bash
# Get worker pod
WORKER_POD=$(kubectl get pods -n $NAMESPACE -l moatless.io/task-id=$TASK_ID | awk '{print $1}')

# Check pod status
kubectl get pod $WORKER_POD -n $NAMESPACE

# View worker logs
kubectl logs $WORKER_POD -n $NAMESPACE --tail=100

# Check agent status on worker
kubectl exec $WORKER_POD -n $NAMESPACE -- curl -s localhost:8001/agent-status

# Main app logs for this task
kubectl logs deployment/moatless-vibe-backend -n $NAMESPACE --tail=200 | grep $TASK_ID
```

---

## Verify Preview Server

### Check Status via API

```bash
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/server/status" | jq .
```

**Response Fields:**
| Field | Description |
|-------|-------------|
| status | `stopped`, `starting`, `started`, `failed` |
| url | External proxy URL (user-facing) |
| pid | Process ID |
| uptimeSeconds | Server uptime |
| error | Error message if failed |
| dirtyContext | Code changed since last restart |

### Start/Stop/Restart

```bash
# Start server
curl -s -X POST -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/server/start"

# Stop server
curl -s -X POST -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/server/stop"

# Restart server
curl -s -X POST -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/server/restart"
```

### Test via kubectl (K8s)

```bash
WORKER_POD=$(kubectl get pods -n $NAMESPACE -l moatless.io/task-id=$TASK_ID | awk '{print $1}')

# Check worker's preview status
kubectl exec $WORKER_POD -n $NAMESPACE -- curl -s localhost:8001/server/status

# Test preview server directly (port 3000)
kubectl exec $WORKER_POD -n $NAMESPACE -- curl -s localhost:3000 | head -20

# Get server logs
kubectl exec $WORKER_POD -n $NAMESPACE -- curl -s "localhost:8001/server/logs?lines=50"
```

### Access via Hostname Proxy

```bash
# Preview accessible at: https://{TASK_ID}.proxy.moatless.dev/
curl -sk "https://${TASK_ID}.proxy.moatless.dev/" -H "Authorization: Bearer $API_KEY" | head -20
```

### Stream Server Logs (SSE)

```bash
curl -N -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/server/logs"
```

---

## Complete E2E Test Script

```bash
#!/bin/bash
set -e

API_KEY="${API_KEY:-mvk_your_api_key}"
BASE_URL="${BASE_URL:-https://moatless.dev}"
NAMESPACE="${NAMESPACE:-vibe}"
TEMPLATE_ID="${TEMPLATE_ID:-your-template-uuid}"

echo "=== 1. Create Vibe Task ==="
RESPONSE=$(curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"agentType\":\"claude-code\",\"templateId\":\"$TEMPLATE_ID\",\"message\":\"Create hello world\",\"start\":true}" \
  "$BASE_URL/api/v1/tasks/vibe")
TASK_ID=$(echo $RESPONSE | jq -r '.taskId')
echo "Task: $TASK_ID"

echo "=== 2. Wait for Worker Pod ==="
sleep 15
kubectl get pods -n $NAMESPACE -l moatless.io/task-id=$TASK_ID
WORKER_POD=$(kubectl get pods -n $NAMESPACE -l moatless.io/task-id=$TASK_ID | awk '{print $1}')

echo "=== 3. Check Agent Status ==="
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/workspace-status" | \
  jq '{agentState, environmentStatus}'

echo "=== 4. Wait for Completion ==="
for i in {1..30}; do
  STATUS=$(curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID" | jq -r '.iterations[-1].status')
  echo "[$i] Status: $STATUS"
  [[ "$STATUS" == "completed" ]] && break
  sleep 5
done

echo "=== 5. Check Preview Server ==="
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/server/status" | jq .

echo "=== 6. Test Preview ==="
kubectl exec $WORKER_POD -n $NAMESPACE -- curl -s localhost:3000 | head -10

echo "=== Done ==="
```
