# Kubernetes Deployment

Push images and deploy to the moatless K8s cluster (Azure AKS).

## Push Images

All push commands cross-compile for `linux/amd64` and push to Docker Hub (`aorwall/` namespace).

```bash
# Push individual components
task push:backend           # Rust cross-compile + push
task push:frontend          # Vite build + nginx + push
task push:sandbox           # Sandbox binary + images + push
task push:servers           # files-server, preview-server + push

# Push frontend + sandbox together
task push

# Push with specific tag
task push:backend TAG=v1.0.0
```

## Deploy with Helm

### Standard Deploy

```bash
helm upgrade vibe k8s/helm/moatless-vibe \
  --namespace vibe \
  -f k8s/helm/values-moatless.yaml
```

### Deploy with Specific Image Tag

```bash
helm upgrade vibe k8s/helm/moatless-vibe \
  --namespace vibe \
  -f k8s/helm/values-moatless.yaml \
  --set image.tag=v1.0.0
```

### Full Deploy (Build + Push + Helm)

```bash
task deploy                 # Build, push, helm upgrade
task deploy TAG=v1.0.0      # With specific tag
```

### Deploy Frontend Only

```bash
task deploy:frontend        # Build, push, restart frontend deployment
```

## Restart Deployments

```bash
# Restart backend (picks up new image)
kubectl rollout restart deployment/moatless-vibe-backend -n vibe
kubectl rollout status deployment/moatless-vibe-backend -n vibe --timeout=300s

# Restart frontend
kubectl rollout restart deployment/vibe-moatless-vibe -n vibe

# Quick restart via Taskfile
task restart
```

## Check Status

```bash
# Overview (pods + deployments + workers)
task status

# Detailed pod status
kubectl get pods -n vibe

# Backend deployment
kubectl get deployment moatless-vibe-backend -n vibe

# Sandbox pods for a specific task
kubectl get pods -n vibe -l moatless.io/task-id=$TASK_ID
```

## View Logs

```bash
# Backend logs
task logs
kubectl logs -n vibe deployment/moatless-vibe-backend --tail=200

# Worker/sandbox logs
task logs:worker

# Specific sandbox pod (multiple containers)
SANDBOX_POD=$(kubectl get pods -n vibe -l moatless.io/task-id=$TASK_ID -o jsonpath='{.items[0].metadata.name}')
kubectl logs -n vibe $SANDBOX_POD -c sandbox --tail=200
kubectl logs -n vibe $SANDBOX_POD -c preview-server --tail=100

# Filter for errors
kubectl logs -n vibe deployment/moatless-vibe-backend --tail=500 | grep -i error
```

## Verify on moatless.dev

### Environment Setup

```bash
# Use the test API key (see setup below)
export API_TOKEN="$TEST_API_KEY"
export BASE_URL="https://moatless.dev"
```

> **Test API Key Setup**: `TEST_API_KEY` must be set in `vibe-secrets`.
> The backend auto-creates a test user on startup. No OAuth required.
> See the `kubernetes-testing` skill for detailed setup steps.

### Smoke Test

```bash
# 1. List repositories
curl -s -H "Authorization: Bearer $API_TOKEN" \
  "$BASE_URL/api/v1/repositories" | jq '.repositories[] | {id, name, kind}'

# 2. Create a test task
TEMPLATE_ID="<template-id>"
TASK_ID=$(curl -s -X POST \
  -H "Authorization: Bearer $API_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"agentType\":\"claude-code\",\"templateId\":\"$TEMPLATE_ID\",\"message\":\"Create a hello world page\",\"start\":true}" \
  "$BASE_URL/api/v1/tasks/vibe" | jq -r '.taskId')
echo "Task: $TASK_ID"

# 3. Wait for completion
while true; do
  TURNS=$(curl -s -H "Authorization: Bearer $API_TOKEN" \
    "$BASE_URL/api/v1/tasks/$TASK_ID" | jq -r '.numTurns')
  echo "Turns: $TURNS"
  [ "$TURNS" != "0" ] && break
  sleep 10
done

# 4. Check workspace status
curl -s -H "Authorization: Bearer $API_TOKEN" \
  "$BASE_URL/api/v1/tasks/$TASK_ID/workspace-status" | jq '.'

# 5. Check for errors in messages
curl -s -H "Authorization: Bearer $API_TOKEN" \
  "$BASE_URL/api/v1/tasks/$TASK_ID/messages" | \
  grep -o "Agent Error" || echo "No agent errors"

# 6. Cleanup
curl -s -X POST -H "Authorization: Bearer $API_TOKEN" \
  "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/cleanup"
curl -s -X DELETE -H "Authorization: Bearer $API_TOKEN" \
  "$BASE_URL/api/v1/tasks/$TASK_ID"
```

### Run E2E Tests Against Production

```bash
BASE_URL=https://moatless.dev GITHUB_PAT=$GITHUB_PAT task test:api
```

### Sandbox-Specific Verification

```bash
# Sandbox provisioning status
curl -s -H "Authorization: Bearer $API_TOKEN" \
  "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/status" | jq '.'

# Environment status
curl -s -H "Authorization: Bearer $API_TOKEN" \
  "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/env-status" | jq '.'

# File tree
curl -s -H "Authorization: Bearer $API_TOKEN" \
  "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/files/tree" | jq '.'

# Server status
curl -s -H "Authorization: Bearer $API_TOKEN" \
  "$BASE_URL/api/sandbox/v1/tasks/$TASK_ID/server/status" | jq '.'
```

## Common Issues

### ImagePullBackOff

**Cause**: Image not pushed or wrong platform (need linux/amd64 for AKS).

**Fix**: Rebuild and push with correct platform:
```bash
task push:backend   # Always cross-compiles to linux/amd64
```

### Pod CrashLoopBackOff

**Diagnose**:
```bash
kubectl describe pod <pod-name> -n vibe
kubectl logs <pod-name> -n vibe --previous
```

### Helm Upgrade Fails

**Check**: Values file syntax and secret references:
```bash
helm template vibe k8s/helm/moatless-vibe -n vibe -f k8s/helm/values-moatless.yaml | head -100
```

### Rollout Stuck

```bash
kubectl rollout status deployment/moatless-vibe-backend -n vibe --timeout=60s
# If stuck, check events:
kubectl describe deployment moatless-vibe-backend -n vibe
```

## Cluster Info

| Resource | Value |
|----------|-------|
| Namespace | `vibe` |
| Helm release | `vibe` |
| Backend deployment | `moatless-vibe-backend` |
| Frontend deployment | `vibe-moatless-vibe` |
| Backend replicas | 2 |
| Production URL | `https://moatless.dev` |
| Ingress | Azure Web App Routing |
| Sandbox labels | `moatless.io/task-id={taskId}` |
