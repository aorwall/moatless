---
name: docker-testing
description: "Test Moatless Vibe in Docker execution mode. Use when running the backend in Docker containers, testing sandbox containers, debugging preview server issues, checking container logs, verifying Docker networking, or troubleshooting container communication. Triggers on: docker testing, docker mode, sandbox containers, preview server docker, container logs, debug docker, dev-docker.sh, docker compose dev"
---

# Docker Testing

Testing workflows for Moatless Vibe in Docker execution mode (local development).

## Quick Reference

### Start Docker Development Environment

```bash
./docker-compose/dev-docker.sh
```

This script:
1. Builds the application (`bun run build:package`)
2. Builds the local sandbox base image (`moatless-vibe-sandbox:local`)
3. Builds the local sandbox-js image (`moatless-vibe-sandbox-js:local`)
4. Starts docker compose with dev overrides

### Architecture

| Component | Container Name | Port | Description |
|-----------|---------------|------|-------------|
| Backend | `moatless-backend` | 8000 | API, orchestration |
| Sandbox | `moatless-moatable-{taskId}` | dynamic | Claude Code execution |
| Redis | `moatless-redis` | 6379 | Event streaming |
| PostgreSQL | `moatless-postgres` | 5432 | Persistent state |

### Key Environment Variables

```bash
SANDBOX_HOST=host.docker.internal  # How backend reaches sandbox containers
WORKSPACE_EXECUTION_MODE=docker    # Use Docker provisioner
```

## Vibe Task Creation

### Create and Start Task

```bash
# Get template ID
curl -s http://localhost:8000/api/v1/repositories | jq '.[0].id'

# Create task (replace TEMPLATE_ID)
curl -s -X POST http://localhost:8000/api/v1/tasks/vibe \
  -H "Content-Type: application/json" \
  -d '{"agentType":"claude-code","templateId":"TEMPLATE_ID","message":"Create hello world","start":true}'
```

### Monitor Task

```bash
TASK_ID="your-task-id"

# Task status
curl -s "http://localhost:8000/api/v1/tasks/$TASK_ID" | jq '{status, numTurns}'

# Workspace status
curl -s "http://localhost:8000/api/v1/tasks/$TASK_ID/workspace-status"
```

## Container Management

### Check Containers

```bash
# All moatless containers
docker ps --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}" | grep moatless

# Sandbox containers only
docker ps --format "{{.Names}}" | grep moatless-moatable
```

### View Logs

```bash
# Backend logs
docker logs moatless-backend --tail 100

# Follow backend logs
docker logs -f moatless-backend

# Filter for errors
docker logs moatless-backend 2>&1 | grep '"level":50'

# Sandbox container logs (replace TASK_ID)
docker logs moatless-moatable-${TASK_ID:0:8}
```

### Restart Backend

```bash
# After code changes, rebuild and restart
bun run build:package
docker compose -f docker-compose/docker-compose.yml \
  -f docker-compose/docker-compose.dev.yml \
  --env-file docker-compose/.env.docker-compose \
  restart backend
```

## Preview Server Debugging

The preview server runs inside sandbox containers. When troubleshooting preview issues:

### Get Preview Server Logs

```bash
# Find sandbox container port
CONTAINER=$(docker ps --format "{{.Names}}" | grep moatless-moatable | head -1)
PORT=$(docker port $CONTAINER 8001 | cut -d: -f2)

# Get preview server logs (shows bun install, bun run dev output)
curl -s "http://localhost:$PORT/server/logs" | jq -r '.logs'

# Get preview server status
curl -s "http://localhost:$PORT/server/status" | jq .
```

### Test Preview Server Connectivity

```bash
# Test inside container
docker exec $CONTAINER curl -s -o /dev/null -w "%{http_code}" http://localhost:3000/

# Test Bun fetch (may differ from curl due to IPv4/IPv6)
docker exec $CONTAINER bun -e "fetch('http://localhost:3000').then(r => console.log(r.status)).catch(e => console.log(e.message))"
```

### Common Preview Server Issues

See `references/preview-troubleshooting.md` for:
- IPv6 vs IPv4 binding issues
- Timeout configuration
- Health check debugging

## Common Issues

### Backend Can't Reach Sandbox Containers

**Symptom**: Timeout errors when starting tasks

**Cause**: Backend uses `localhost` but Docker containers use different network

**Fix**: Ensure `SANDBOX_HOST=host.docker.internal` in docker-compose.yml

### Preview Server Timeout

**Symptom**: "Server process started but did not become HTTP-responsive"

**Causes**:
1. Dev server binding to IPv6 only (not IPv4)
2. Timeout too short for `bun install` + `bun run dev`

**Fixes**:
1. Add `--host` flag to Vite/dev server command
2. Increase `startupTimeoutSeconds` in repository config (default: 90s)

### ImagePullBackOff for Sandbox

**Cause**: Sandbox image not available locally

**Fix**:
```bash
docker pull ghcr.io/aorwall/moatless-vibe-sandbox-js:latest
```

### Container Not Starting

```bash
# Check container events
docker inspect moatless-backend | jq '.[0].State'

# Check docker compose logs
docker compose -f docker-compose/docker-compose.yml \
  -f docker-compose/docker-compose.dev.yml \
  logs backend
```

## Health Checks

```bash
# Backend health
curl -s http://localhost:8000/health | jq .

# Sandbox health (replace PORT)
curl -s http://localhost:$PORT/health | jq .
```

## Cleanup

```bash
# Stop all containers
docker compose -f docker-compose/docker-compose.yml \
  -f docker-compose/docker-compose.dev.yml \
  --env-file docker-compose/.env.docker-compose \
  down

# Remove sandbox containers
docker rm -f $(docker ps -aq --filter "name=moatless-moatable")

# Clean volumes (WARNING: deletes data)
docker compose -f docker-compose/docker-compose.yml down -v
```

## Resources

- `references/preview-troubleshooting.md` - Detailed preview server debugging
- `references/networking.md` - Docker networking details
