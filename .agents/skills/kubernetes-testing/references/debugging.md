# Kubernetes Debugging

Detailed debugging commands for K8s mode.

## Pod Status Reference

| Status | Meaning |
|--------|---------|
| `Pending` | Waiting for resources or image pull |
| `Running` | Pod running and healthy |
| `ImagePullBackOff` | Failed to pull Docker image |
| `CrashLoopBackOff` | Container keeps crashing |
| `Completed` | Pod finished successfully |
| `Error` | Pod failed |

## Log Filtering

```bash
# Errors only
kubectl logs deployment/moatless-vibe-backend -n $NAMESPACE --tail=200 | grep '"level":"ERROR"'

# Warnings and errors
kubectl logs deployment/moatless-vibe-backend -n $NAMESPACE --tail=200 | grep -E '"level":"(WARN|ERROR)"'

# Specific task
kubectl logs deployment/moatless-vibe-backend -n $NAMESPACE --tail=200 | grep $TASK_ID

# Parse JSON with jq
kubectl logs deployment/moatless-vibe-backend -n $NAMESPACE --tail=50 | \
  jq -r 'select(.taskId == "your-task-id") | "\(.time) \(.msg)"'
```

## Pod Diagnostics

```bash
# Pod events
kubectl describe pod <pod-name> -n $NAMESPACE | grep -A 10 "Events:"

# Node resources
kubectl describe nodes | grep -A 5 "Allocated resources"

# PVC status
kubectl get pvc -n $NAMESPACE
```

## Health Checks

```bash
# Main app
kubectl exec deployment/moatless-vibe-backend -n $NAMESPACE -- curl -s localhost:8080/health

# Worker pod
WORKER_IP=$(kubectl get pod $WORKER_POD -n $NAMESPACE -o jsonpath='{.status.podIP}')
kubectl exec deployment/moatless-vibe-backend -n $NAMESPACE -- curl -s http://$WORKER_IP:8000/health
```

## Regular Task Creation

```bash
# Get repository ID
REPO_ID=$(curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/repositories" | jq -r '.repositories[0].id')

# Create task with auto-start
curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"agentType\":\"claude-code\",\"repositoryId\":\"$REPO_ID\",\"message\":\"Create hello.txt\",\"start\":true}" \
  "$BASE_URL/api/v1/tasks"
```

## Send Follow-up Messages

```bash
curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"message":"Now update the file"}' \
  "$BASE_URL/api/v1/tasks/$TASK_ID/messages"
```

## Preview Server Testing

```bash
# Get worker pod
WORKER_POD=$(kubectl get pods -n $NAMESPACE -l moatless.io/task-id=$TASK_ID -o jsonpath='{.items[0].metadata.name}')

# Check preview server status (backend-proxied; the container itself only
# serves /health and /livez)
curl -s -H "Authorization: Bearer $API_KEY" \
  "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/server/status" | jq '.servers'

# Test the dev server directly inside the sandbox container
kubectl exec -n $NAMESPACE $WORKER_POD -c sandbox -- curl -s localhost:3000 | head -20

# Via the preview URL reported in server status
PREVIEW_URL=$(curl -s -H "Authorization: Bearer $API_KEY" \
  "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/server/status" | jq -r '.servers[] | select(.default) | .url')
curl -sk "$PREVIEW_URL" -H "Authorization: Bearer $API_KEY"
```

## Troubleshooting Checklist

1. **Pod not starting?**
   - Check events: `kubectl describe pod <name> -n vibe`
   - Check image platform (ARM vs AMD64)
   - Check resource limits

2. **Agent failing?**
   - Check worker logs
   - Verify API keys configured
   - Check workspace status for errors

3. **After backend restart?**
   - Worker pods continue running
   - New messages re-register targets
   - Check for "re-registered target" in logs

4. **GitHub auth errors (vibe tasks)?**
   - Connect GitHub in Settings
   - Ensure token has `repo` scope
