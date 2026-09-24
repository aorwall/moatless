# Kubernetes Testing Guide

This guide explains how to test the Moatless Vibe application in Kubernetes mode, including debugging workflows, checking logs, and verifying pod status.

## Prerequisites

- `kubectl` configured with access to the cluster
- API key for authentication (see [API Authentication Guide](./api-authentication.md))
- Access to the `vibe` namespace

## Environment Setup

```bash
export API_KEY="mvk_your_api_key"
export BASE_URL="https://moatless.xyz"
export NAMESPACE="vibe"
```

## Architecture Overview

In Kubernetes mode, the application consists of:

| Component | Description |
|-----------|-------------|
| **Main App Pod** | `vibe-moatless-vibe-*` - Handles API requests, orchestrates tasks |
| **Worker Pods** | `moatless-moatless-vibe-{taskId}-*` - Execute Claude Code agents |
| **Database** | PostgreSQL for persistent state |

### Key Components

- **WorkspaceRuntimeManager**: Manages workspace provisioning and pod lifecycle
- **KubernetesEnvironmentDriver**: Handles agent creation and communication with worker pods
- **executionTargets Map**: In-memory mapping of taskId → pod details (lost on restart)

## Testing Workflows

### 1. Create a Vibe Task (Recommended)

Vibe tasks use templates for quick prototyping with pre-configured Scripts.

**Important: K8s Mode Behavior**

In Kubernetes mode, vibe tasks automatically **fork templates to GitHub** instead of cloning locally:

```
Local Mode:  Template → Clone to local disk → Local repository
K8s Mode:    Template → Create GitHub repo → Push code → Remote repository
```

This ensures K8s worker pods can access the repository since they can't use local paths.

**Requirements for K8s mode vibe tasks:**
- GitHub token configured in Settings
- User must have GitHub repo creation permissions
- The forked repo is created as a private repository by default

```bash
# List available templates
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/repositories" | jq '.repositories[] | select(.kind == "template") | {id, name}'

# Create a vibe task with a template (use the template UUID)
# In K8s mode, this automatically forks the template to GitHub
TEMPLATE_ID="f7a613fc-c769-4740-b7be-b4d57d20b9f0"  # moatable template

TASK_RESPONSE=$(curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"agentType\":\"claude-code\",\"templateId\":\"$TEMPLATE_ID\",\"message\":\"Create a simple hello world page\",\"start\":true}" \
  "$BASE_URL/api/v1/tasks/vibe")

TASK_ID=$(echo $TASK_RESPONSE | jq -r '.taskId')
echo "Created vibe task: $TASK_ID"

# Check the created repository (should be 'remote' kind in K8s mode)
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID" | jq '.repository | {kind, name, url: .remote.url}'
```

### 2. Create a Regular Task

```bash
# Get repository ID
REPO_ID=$(curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/repositories" | jq -r '.repositories[0].id')

# Create a task with auto-start
TASK_RESPONSE=$(curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"agentType\":\"claude-code\",\"repositoryId\":\"$REPO_ID\",\"message\":\"Create a hello.txt file\",\"start\":true}" \
  "$BASE_URL/api/v1/tasks")

TASK_ID=$(echo $TASK_RESPONSE | jq -r '.taskId')
echo "Created task: $TASK_ID"
```

### 3. Monitor Task Status

```bash
# Check task status
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID" | jq '{status, numTurns, iterations: [.iterations[] | {iterationNumber, status, result}]}'

# Check workspace status
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/workspace-status" | jq .
```

### 4. Send Follow-up Messages

```bash
# Send a follow-up message
curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"message":"Now update the file to say Goodbye"}' \
  "$BASE_URL/api/v1/tasks/$TASK_ID/messages"
```

### 5. Test a Script

A vibe task's template can declare a `runOnStart` Script, which starts once the
sandbox reports Ready. Check it with `moat tasks scripts list <task-id>`, and
read its output in the T3 UI terminal or via `moat tasks scripts run <name> <task-id>`
— not through a worker-pod HTTP endpoint.

```bash
# Get the worker pod name
WORKER_POD=$(kubectl get pods -n $NAMESPACE | grep ${TASK_ID:0:8} | awk '{print $1}')

# Test the Script's port directly on the pod (adjust the port to the Script's own)
kubectl exec -n $NAMESPACE $WORKER_POD -- curl -s localhost:3000 | head -20

# Test via hostname routing (requires authentication)
# The preview is accessible at: https://{TASK_ID}.proxy.moatless.xyz/
curl -sk "https://${TASK_ID}.proxy.moatless.xyz/" -H "Authorization: Bearer $API_KEY"
```

## Kubernetes Debugging

### Checking Pods

```bash
# List all pods in the vibe namespace
kubectl get pods -n $NAMESPACE

# List worker pods for a specific task
kubectl get pods -n $NAMESPACE | grep ${TASK_ID:0:8}

# Get detailed pod info
kubectl describe pod <pod-name> -n $NAMESPACE

# Check pod events (useful for debugging startup issues)
kubectl get events -n $NAMESPACE --sort-by='.lastTimestamp' | tail -20
```

### Pod Status Reference

| Status | Meaning |
|--------|---------|
| `Pending` | Pod scheduled, waiting for resources or image pull |
| `Running` | Pod is running and healthy |
| `ImagePullBackOff` | Failed to pull Docker image |
| `CrashLoopBackOff` | Container keeps crashing |
| `Completed` | Pod finished successfully |
| `Error` | Pod failed with an error |

### Checking Logs

#### Main Application Logs

```bash
# Recent logs
kubectl logs deployment/vibe-moatless-vibe -n $NAMESPACE --tail=100

# Follow logs in real-time
kubectl logs deployment/vibe-moatless-vibe -n $NAMESPACE -f

# Filter logs for a specific task
kubectl logs deployment/vibe-moatless-vibe -n $NAMESPACE --tail=200 | grep $TASK_ID
```

#### Worker Pod Logs

```bash
# Get worker pod name
WORKER_POD=$(kubectl get pods -n $NAMESPACE | grep ${TASK_ID:0:8} | awk '{print $1}')

# View worker logs
kubectl logs $WORKER_POD -n $NAMESPACE

# Follow worker logs
kubectl logs $WORKER_POD -n $NAMESPACE -f
```

### Log Levels

The application uses pino logging with these levels:

| Level | Number | Description |
|-------|--------|-------------|
| `trace` | 10 | Very detailed tracing |
| `debug` | 20 | Debug information |
| `info` | 30 | Normal operation info |
| `warn` | 40 | Warnings |
| `error` | 50 | Errors |
| `fatal` | 60 | Fatal errors |

**Note**: Production typically runs at `info` level (30), so `debug` messages won't appear.

### Filtering Logs

```bash
# Filter for errors only
kubectl logs deployment/vibe-moatless-vibe -n $NAMESPACE --tail=200 | grep '"level":50'

# Filter for warnings and errors
kubectl logs deployment/vibe-moatless-vibe -n $NAMESPACE --tail=200 | grep -E '"level":(40|50)'

# Filter for specific log messages
kubectl logs deployment/vibe-moatless-vibe -n $NAMESPACE --tail=200 | grep -E "Agent execution completed|No execution target|Ensuring Kubernetes"

# Parse JSON logs with jq
kubectl logs deployment/vibe-moatless-vibe -n $NAMESPACE --tail=50 | jq -r 'select(.taskId == "your-task-id") | "\(.time) \(.msg)"'
```

## Common Issues and Debugging

### "GitHub authentication required" (Vibe Tasks)

**Cause**: Trying to create a vibe task in K8s mode without a GitHub token configured.

**Resolution**:
1. Go to Settings in the UI
2. Connect your GitHub account
3. Ensure the token has `repo` scope for creating repositories

### "Local repositories not supported in Kubernetes mode"

**Cause**: A task references a local repository path. K8s worker pods cannot access the host filesystem.

**For vibe tasks**: This should not occur if using templates - they automatically fork to GitHub.

**For regular tasks**: Use a remote repository (GitHub, GitLab, etc.) instead of a local path.

### "No execution target registered for task"

**Cause**: The in-memory `executionTargets` Map was cleared (usually after backend restart) but the pod still exists.

**Diagnosis**:
```bash
# Check if worker pod exists
kubectl get pods -n $NAMESPACE | grep ${TASK_ID:0:8}

# Check main app logs for registration
kubectl logs deployment/vibe-moatless-vibe -n $NAMESPACE --tail=100 | grep -E "re-registered|registerTarget"
```

**Resolution**: This was fixed by re-registering targets when reusing healthy pods. If it occurs, restart the main app pod.

### "ImagePullBackOff"

**Cause**: Kubernetes can't pull the Docker image.

**Diagnosis**:
```bash
# Check pod events
kubectl describe pod <pod-name> -n $NAMESPACE | grep -A 10 "Events:"

# Common causes:
# - Wrong platform (ARM vs AMD64)
# - Image doesn't exist
# - Registry authentication issues
```

**Resolution**:
```bash
# For platform issues, rebuild with correct architecture
docker buildx build --platform linux/amd64 -t aorwall/moatless-frontend:latest -f docker/Dockerfile.app --push .
```

### "Pod stuck in Pending"

**Diagnosis**:
```bash
# Check pod events
kubectl describe pod <pod-name> -n $NAMESPACE

# Check node resources
kubectl describe nodes | grep -A 5 "Allocated resources"

# Check if PVC is bound (if using persistent storage)
kubectl get pvc -n $NAMESPACE
```

### Agent Not Responding

**Diagnosis**:
```bash
# Check worker pod status
kubectl get pod $WORKER_POD -n $NAMESPACE

# Check worker logs for errors
kubectl logs $WORKER_POD -n $NAMESPACE | tail -50

# Check if worker can reach the main API
kubectl exec $WORKER_POD -n $NAMESPACE -- curl -s http://vibe-moatless-vibe:8000/health
```

### Workspace Status Shows Error

```bash
# Check workspace status via API
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/workspace-status" | jq .

# Look for environmentError field
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/workspace-status" | jq '{environmentStatus, environmentError}'
```

## Health Checks

### Check Main App Health

```bash
# Via kubectl
kubectl exec deployment/vibe-moatless-vibe -n $NAMESPACE -- curl -s localhost:8000/health

# Via external URL
curl -s "$BASE_URL/health"
```

### Check Worker Pod Health

```bash
# Get worker pod IP
WORKER_IP=$(kubectl get pod $WORKER_POD -n $NAMESPACE -o jsonpath='{.status.podIP}')

# Health check from main app pod
kubectl exec deployment/vibe-moatless-vibe -n $NAMESPACE -- curl -s http://$WORKER_IP:8000/health
```

## Deployment Operations

### Restart Main Application

```bash
# Trigger rolling restart
kubectl rollout restart deployment/vibe-moatless-vibe -n $NAMESPACE

# Watch rollout status
kubectl rollout status deployment/vibe-moatless-vibe -n $NAMESPACE

# Check new pod is running
kubectl get pods -n $NAMESPACE -l app.kubernetes.io/name=moatless-vibe
```

### Delete Worker Pod (Force Cleanup)

```bash
# Delete specific worker pod
kubectl delete pod $WORKER_POD -n $NAMESPACE

# Delete all worker pods for a task
kubectl delete pods -n $NAMESPACE -l task-id=${TASK_ID:0:8}
```

### Scale Deployment

```bash
# Scale to 0 (stop)
kubectl scale deployment/vibe-moatless-vibe -n $NAMESPACE --replicas=0

# Scale back up
kubectl scale deployment/vibe-moatless-vibe -n $NAMESPACE --replicas=1
```

## End-to-End Test Script

```bash
#!/bin/bash
# e2e-k8s-test.sh - End-to-end Kubernetes test

set -e

API_KEY="${API_KEY:-mvk_your_api_key}"
BASE_URL="${BASE_URL:-https://moatless.xyz}"
NAMESPACE="${NAMESPACE:-vibe}"

echo "=== E2E Kubernetes Test ==="

# 1. Check main app is running
echo "1. Checking main app..."
kubectl get pods -n $NAMESPACE -l app.kubernetes.io/name=moatless-vibe

# 2. Get repository
echo "2. Getting repository..."
REPO_ID=$(curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/repositories" | jq -r '.repositories[0].id')
echo "   Repository: $REPO_ID"

# 3. Create task
echo "3. Creating task..."
TASK_RESPONSE=$(curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"agentType\":\"claude-code\",\"repositoryId\":\"$REPO_ID\",\"message\":\"Create a test.txt file with 'E2E Test'\",\"start\":true}" \
  "$BASE_URL/api/v1/tasks")
TASK_ID=$(echo $TASK_RESPONSE | jq -r '.taskId')
echo "   Task ID: $TASK_ID"

# 4. Wait for worker pod
echo "4. Waiting for worker pod..."
sleep 10
kubectl get pods -n $NAMESPACE | grep ${TASK_ID:0:8} || echo "   Worker pod not yet created"

# 5. Wait for completion
echo "5. Waiting for task completion..."
for i in {1..30}; do
  STATUS=$(curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID" | jq -r '.iterations[-1].status')
  echo "   [$i] Status: $STATUS"
  if [[ "$STATUS" == "completed" ]]; then
    break
  fi
  sleep 5
done

# 6. Send follow-up
echo "6. Sending follow-up message..."
curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"message":"Update test.txt to say E2E Test Passed"}' \
  "$BASE_URL/api/v1/tasks/$TASK_ID/messages"

# 7. Wait for second iteration
echo "7. Waiting for second iteration..."
sleep 30
RESULT=$(curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID" | jq -r '.iterations[-1].result')
echo "   Result: $RESULT"

# 8. Check for errors
echo "8. Checking logs for errors..."
kubectl logs deployment/vibe-moatless-vibe -n $NAMESPACE --tail=50 | grep -E "$TASK_ID.*(error|Error|failed)" || echo "   No errors found!"

echo "=== Test Complete ==="
```

## Useful kubectl Aliases

Add these to your `~/.bashrc` or `~/.zshrc`:

```bash
alias k='kubectl'
alias kv='kubectl -n vibe'
alias kvlogs='kubectl logs deployment/vibe-moatless-vibe -n vibe'
alias kvpods='kubectl get pods -n vibe'
alias kvdesc='kubectl describe pod -n vibe'
```

## Troubleshooting Checklist

1. **Pod not starting?**
   - Check events: `kubectl describe pod <name> -n vibe`
   - Check image exists and correct platform
   - Check resource limits

2. **Agent failing?**
   - Check worker logs: `kubectl logs <worker-pod> -n vibe`
   - Check main app logs for errors
   - Verify API keys and tokens are configured

3. **Messages not being processed?**
   - Check task status via API
   - Check workspace status for errors
   - Verify worker pod is running

4. **After backend restart?**
   - Worker pods should continue running
   - New messages should re-register targets automatically
   - Check for "re-registered target" in debug logs

## Related Documentation

- [API Authentication Guide](./api-authentication.md)
- [Sandbox Lifecycle](./sandbox-lifecycle.md)
