# Troubleshooting Guide

Cross-environment debugging patterns for Docker Compose and Kubernetes.

## General Approach

1. **Identify the environment**: Docker Compose (local) or K8s (production)
2. **Identify the component**: Backend, sandbox, frontend, database, or network
3. **Check logs** for the affected component
4. **Reproduce** in the simplest environment possible (prefer Docker Compose for debugging)

## Backend (Rust/Axum)

### Log Format

Backend uses `tracing` with JSON structured logging. Key fields: `level`, `message`, `target`, `span`.

### Docker Compose

```bash
# All backend logs
docker logs moatless-backend --tail 200

# Follow logs
docker logs moatless-backend -f

# Filter by level
docker logs moatless-backend 2>&1 | grep '"level":"ERROR"'
docker logs moatless-backend 2>&1 | grep '"level":"WARN"'

# Filter by module
docker logs moatless-backend 2>&1 | grep 'sandbox'
docker logs moatless-backend 2>&1 | grep 'grpc'
```

### Kubernetes

```bash
# Backend logs
kubectl logs -n vibe deployment/moatless-vibe-backend --tail=200

# Follow
kubectl logs -n vibe deployment/moatless-vibe-backend -f --tail=50

# Specific pod (when multiple replicas)
kubectl logs -n vibe moatless-vibe-backend-<hash> --tail=200

# Previous crash logs
kubectl logs -n vibe <pod-name> --previous
```

### Common Backend Errors

| Error Pattern | Likely Cause | Fix |
|--------------|-------------|-----|
| `Sandbox unreachable` | Service routing issue or pod not ready | Check sandbox pod status, readiness probes |
| `database connection` | PostgreSQL down or connection pool exhausted | Check PG health, connection limits |
| `Redis connection` | Redis down or unreachable | Check Redis container/service |
| `gRPC error` | Sandbox pod crashed or network issue | Check sandbox pod logs |
| `JWT verification failed` | Token expired or wrong secret | Check `TASK_TOKEN_SECRET` |

## Sandbox (Rust Agent)

### Docker Compose

```bash
# Find sandbox containers
docker ps --format "{{.Names}}" | grep moatless-sandbox

# Sandbox logs
docker logs moatless-sandbox-${TASK_ID} --tail 200

# Exec into sandbox
docker exec -it moatless-sandbox-${TASK_ID} bash
```

### Kubernetes

```bash
# Find sandbox pod
kubectl get pods -n vibe -l moatless.io/task-id=$TASK_ID

# Sandbox container logs (main container)
SANDBOX_POD=$(kubectl get pods -n vibe -l moatless.io/task-id=$TASK_ID -o jsonpath='{.items[0].metadata.name}')
kubectl logs -n vibe $SANDBOX_POD -c sandbox --tail=200

# Sidecar container logs (preview-server, files-server)
kubectl logs -n vibe $SANDBOX_POD -c preview-server --tail=100
kubectl logs -n vibe $SANDBOX_POD -c files-server --tail=100

# All containers in pod
kubectl describe pod $SANDBOX_POD -n vibe

# Exec into sandbox container
kubectl exec -n vibe -it $SANDBOX_POD -c sandbox -- bash
```

### Common Sandbox Errors

| Error Pattern | Likely Cause | Fix |
|--------------|-------------|-----|
| `Claude CLI not found` | Image missing claude CLI | Rebuild sandbox image |
| `Permission denied` | File ownership (needs UID 1000) | Check volume mounts, fsGroup |
| `Agent state: Error` | Claude CLI crashed | Check sandbox logs for stderr |
| `gRPC connection refused` | Backend unreachable from sandbox | Check network, backend URL |
| `OOMKilled` | Container exceeded memory limit | Increase resource limits in Helm values |

## Frontend (React)

### Local Development

Run frontend locally with hot-reload, pointing to Docker Compose backend:

```bash
task dev
# Frontend at http://localhost:3000 (Vite dev server)
# API proxied to http://localhost:8080 (Docker Compose backend)
```

### Browser Debugging with Playwright CLI

Use the `playwright-cli` skill for browser-based debugging:

**Against local dev server:**
- Navigate to `http://localhost:3000`
- Inspect components, take screenshots, check accessibility

**Against moatless.dev:**
- Navigate to `https://moatless.dev`
- Verify deployed UI changes
- Take screenshots for comparison
- Check console errors

### Common Frontend Issues

| Issue | Debug Approach |
|-------|---------------|
| API errors | Check Network tab, verify backend is running |
| Stale data | TanStack Query cache — hard refresh or invalidate |
| Routing issues | Check TanStack Router file-based routes |
| Style issues | Check Tailwind v4 classes, inspect in browser |
| Type errors | `bun run type:check` for TypeScript issues |

## Database (PostgreSQL)

### Docker Compose

```bash
# Check health
docker exec moatless-postgres pg_isready

# Connect
docker exec -it moatless-postgres psql -U postgres -d moatless

# Check tables
docker exec moatless-postgres psql -U postgres -d moatless -c '\dt'

# Query tasks
docker exec moatless-postgres psql -U postgres -d moatless -c "SELECT task_id, status FROM tasks LIMIT 10;"
```

### Kubernetes

```bash
# Port-forward to PostgreSQL
kubectl port-forward -n vibe svc/moatless-postgres 5432:5432

# Connect locally
psql -h localhost -U postgres -d moatless
```

## Network Issues

### Docker Compose

```bash
# Check Docker network
docker network inspect moatless_default

# Test backend→sandbox connectivity
docker exec moatless-backend curl -s http://host.docker.internal:<sandbox-port>/health

# Check port bindings
docker ps --format "{{.Names}}\t{{.Ports}}" | grep moatless
```

### Kubernetes

```bash
# Check service endpoints
kubectl get endpoints -n vibe | grep moatless

# Check service DNS resolution
kubectl run -n vibe debug --rm -it --image=busybox -- nslookup moatless-ws-${TASK_ID}

# Check network policies
kubectl get networkpolicy -n vibe
```

## Reproducing Issues

### From Production to Local

1. **Identify the failing scenario** on moatless.dev
2. **Check production logs**: `task logs` or `kubectl logs`
3. **Reproduce locally**:
   ```bash
   task compose:up
   # Create similar task via API on localhost:8080
   ```
4. **Debug with full logs**: Docker Compose gives easier access to all container logs
5. **Fix and verify locally** before deploying

### From Bug Report to Spec

1. **Reproduce** the bug in Docker Compose
2. **Write a failing spec** in the appropriate `specs/` file
3. **Write the test** in `tests/api/chains/`
4. **Fix the code**
5. **Verify the test passes**: `task test:api:single -- <chain>`
6. **Deploy and verify** on moatless.dev
