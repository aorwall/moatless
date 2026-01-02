# Kubernetes Integration Tests

Integration tests for `KubernetesEnvironmentDriver` that run against real worker pods.

## Prerequisites

- Kubernetes cluster access (kubectl configured)
- RBAC permissions to create/delete pods in `vibe` namespace
- Worker image available in registry

## Running Tests

### Option 1: Run as K8s Job (Recommended)

```bash
# With full worker (~1.7GB)
kubectl apply -f apps/backend/src/features/workspace/drivers/__tests__/k8s-integration-test-job.yaml

# With slim JS worker (~530MB, faster)
kubectl apply -f apps/backend/src/features/workspace/drivers/__tests__/k8s-integration-test-job-slim.yaml
```

**Monitor:**
```bash
kubectl logs -n vibe -l job-name=k8s-driver-integration-test -f
```

**Cleanup:**
```bash
kubectl delete job k8s-driver-integration-test -n vibe
kubectl delete job k8s-driver-integration-test-slim -n vibe
```

### Option 2: Run Locally

Requires direct pod IP access (VPN/port-forward/in-cluster):

```bash
K8S_ENABLED=true \
K8S_NAMESPACE=vibe \
K8S_WORKER_IMAGE=aorwall/moatless-vibe-worker:latest \
NODE_TLS_REJECT_UNAUTHORIZED=0 \
bun test apps/backend/src/features/workspace/drivers/__tests__/kubernetes-environment-driver.integration.test.ts
```

## Rebuilding Test Runner

```bash
./docker/build-test-runner.sh
```

## Test Coverage

| Category | Tests |
|----------|-------|
| File Operations | readFile, writeFile, listFiles |
| Command Execution | run commands, exit codes |
| Git Operations | status, changes |
| Search | file search by query |

## Timeouts

- Pod creation: Up to 5 minutes
- Test execution: ~30 seconds
- Total: ~1-2 minutes with cached images
