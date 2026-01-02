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
kubectl logs deployment/vibe-moatless-vibe -n $NAMESPACE --tail=200 | grep '"level":50'

# Warnings and errors
kubectl logs deployment/vibe-moatless-vibe -n $NAMESPACE --tail=200 | grep -E '"level":(40|50)'

# Specific task
kubectl logs deployment/vibe-moatless-vibe -n $NAMESPACE --tail=200 | grep $TASK_ID

# Parse JSON with jq
kubectl logs deployment/vibe-moatless-vibe -n $NAMESPACE --tail=50 | \
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
kubectl exec deployment/vibe-moatless-vibe -n $NAMESPACE -- curl -s localhost:8000/health

# Worker pod
WORKER_IP=$(kubectl get pod $WORKER_POD -n $NAMESPACE -o jsonpath='{.status.podIP}')
kubectl exec deployment/vibe-moatless-vibe -n $NAMESPACE -- curl -s http://$WORKER_IP:8000/health
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
WORKER_POD=$(kubectl get pods -n $NAMESPACE | grep ${TASK_ID:0:8} | awk '{print $1}')

# Check preview server status
kubectl exec -n $NAMESPACE $WORKER_POD -- curl -s localhost:8001/server/status

# Test preview directly
kubectl exec -n $NAMESPACE $WORKER_POD -- curl -s localhost:3000 | head -20

# Via hostname routing
curl -sk "https://${TASK_ID}.proxy.moatless.ai/" -H "Authorization: Bearer $API_KEY"
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
