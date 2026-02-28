# Kubernetes Integration Tests

Integration tests for `KubernetesEnvironmentDriver` that run against real worker pods.

> **Note**: For testing vibe tasks in K8s mode (which fork templates to GitHub), see the [Kubernetes Testing Guide](./kubernetes-testing-guide.md).

## Prerequisites

- Kubernetes cluster access (kubectl configured)
- RBAC permissions to create/delete pods in the `vibe` namespace
- Worker image available in registry

## Running Tests

### Option 1: Run as K8s Job (Recommended)

Tests run inside the cluster with direct pod network access.

```bash
# With full worker (~1.7GB)
kubectl apply -f apps/backend/src/features/workspace/drivers/__tests__/k8s-integration-test-job.yaml

# With slim JS worker (~530MB, faster)
kubectl apply -f apps/backend/src/features/workspace/drivers/__tests__/k8s-integration-test-job-slim.yaml
```

**Monitor progress:**
```bash
kubectl logs -n vibe -l job-name=k8s-driver-integration-test -f
# or for slim:
kubectl logs -n vibe -l job-name=k8s-driver-integration-test-slim -f
```

**Check result:**
```bash
kubectl get jobs -n vibe | grep integration-test
```

**Cleanup:**
```bash
kubectl delete job k8s-driver-integration-test -n vibe
kubectl delete job k8s-driver-integration-test-slim -n vibe
```

### Option 2: Run Locally (requires cluster network access)

Only works if you have direct access to pod IPs (VPN, port-forward, or in-cluster).

```bash
K8S_ENABLED=true \
K8S_NAMESPACE=vibe \
K8S_SANDBOX_IMAGE=aorwall/moatless-vibe-sandbox:latest \
NODE_TLS_REJECT_UNAUTHORIZED=0 \
bun test apps/backend/src/features/workspace/drivers/__tests__/kubernetes-environment-driver.integration.test.ts
```

## Rebuilding Test Runner Image

After changing test code:

```bash
./docker/build-test-runner.sh
```

This builds and pushes `aorwall/moatless-vibe-test-runner:latest`.

## Test Coverage

The integration tests verify:

| Category | Tests |
|----------|-------|
| File Operations | readFile, writeFile, listFiles (recursive) |
| Command Execution | run commands, exit codes, working directory |
| Git Operations | status, changes, detecting modifications |
| Search | file search by query |
| Error Handling | non-existent paths |

## Timeouts

- **Pod creation**: Up to 5 minutes (image pull for large images)
- **Test execution**: ~30 seconds after pod is ready
- **Total job time**: ~1-2 minutes with cached images

## Troubleshooting

**Pod stuck in Pending:**
```bash
kubectl describe pod -n vibe -l app=k8s-integration-test
kubectl get events -n vibe --sort-by=.metadata.creationTimestamp | tail -20
```

**Worker pod crashes:**
```bash
kubectl logs -n vibe <worker-pod-name>
```

**RBAC issues:**
```bash
kubectl auth can-i create pods -n vibe --as=system:serviceaccount:vibe:k8s-test-runner
```
