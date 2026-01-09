---
description: Build, push, and deploy dev images to moatless K8s environment
allowed-tools: Bash, Read
---

# Deploy Dev Images to Kubernetes

Build Docker images locally, push to GHCR, deploy to the moatless environment, and verify the deployment.

## Arguments

- `$ARGUMENTS` - Optional flags: `--tag <name>`, `--app-only`, `--skip-build`

## Workflow

### Step 1: Run Deploy Script

Execute the deploy script with any provided arguments:

```bash
./scripts/deploy-dev.sh $ARGUMENTS
```

If no `--tag` is provided, it will auto-generate one using git SHA and timestamp.

### Step 2: Verify Deployment

After deployment, verify the pod is running with the correct image:

```bash
# Check pod status
kubectl get pods -n vibe -l app.kubernetes.io/name=moatless-vibe

# Verify image tag
kubectl get deployment moatless-vibe -n vibe -o jsonpath='{.spec.template.spec.containers[0].image}'
```

### Step 3: Health Check

Verify the application is responding:

```bash
# Check frontend responds
curl -s -o /dev/null -w "%{http_code}" https://moatless.xyz/

# Check API responds (401 = auth working)
curl -s https://moatless.xyz/api/v1/feature-flags | head -c 100
```

### Step 4: Check Logs for Errors

Look for any startup errors:

```bash
kubectl logs deployment/moatless-vibe -n vibe --tail=20 | grep -E '"level":"(info|error)"'
```

## Success Criteria

Report deployment as successful when:
- Pod is `1/1 Running`
- Image tag matches the deployed tag
- Frontend returns HTTP 200
- No critical errors in logs (ignore ZodError for workspace snapshots - that's pre-existing data)

## Example Usage

```
/deploy-dev                        # Auto-generated tag
/deploy-dev --tag my-feature       # Custom tag
/deploy-dev --app-only             # Skip worker image
/deploy-dev --skip-build           # Deploy existing image only
```
