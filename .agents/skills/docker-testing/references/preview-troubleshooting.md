# Preview Server Troubleshooting

Server logs/status are served by the **backend proxy** (`localhost:8080` in
Docker mode), not the sandbox container — the container's own HTTP server only
exposes `/health` and `/livez`. Server start config (commands, timeout) lives on
the task's **workspace** (`moat workspaces …`), not on the repository.

## Common Issues

### Server Process Started But Not HTTP-Responsive

**Error**: `Server process started but did not become HTTP-responsive within timeout`

**Root Causes**:

1. **IPv6 vs IPv4 binding** (most common)
2. Timeout too short for `bun install` + `bun run dev`
3. Dev server startup error

### IPv6/IPv4 Binding Issue

**Symptom**: Health check fails, but dev server is running.

**Cause**: Vite binds to IPv6 (`::1`) by default. Bun's `fetch()` tries IPv4 first, fails to connect.

**Diagnosis**:
```bash
CONTAINER=$(docker ps --format "{{.Names}}" | grep moatless-sandbox | head -1)

# Preview server logs (SSE stream) — look for the bound address
moat tasks logs <task_id> --server preview
# Look for: "Local: http://localhost:3000/" vs "http://[::1]:3000/"

# Test IPv4 (will fail)
docker exec $CONTAINER bun -e "fetch('http://localhost:3000').then(r => console.log(r.status)).catch(e => console.log('IPv4 failed:', e.message))"

# Test IPv6 (will succeed)
docker exec $CONTAINER bun -e "fetch('http://[::1]:3000').then(r => console.log(r.status)).catch(e => console.log('IPv6 failed:', e.message))"
```

**Fix**: Add `--host` to the dev server command. Start commands live on the
workspace's server config, so override the workspace (not the repo):
```bash
moat workspaces override <workspace_id> \
  --servers-json '[{"name":"preview","port":3000,"default":true,"startCommands":["bun run dev -- --host"]}]'
```

### Timeout Issues

**Default timeout**: 90 seconds (per-server `startupTimeoutSeconds`).

**Diagnosis**:
```bash
# Current server config, including startupTimeoutSeconds
moat workspaces get <workspace_id> | jq '.servers'
```

**Fix**: Increase the timeout if `bun install` takes too long (set it alongside
the other server fields so the override is complete):
```bash
moat workspaces override <workspace_id> \
  --servers-json '[{"name":"preview","port":3000,"default":true,"startCommands":["bun run dev -- --host"],"startupTimeoutSeconds":120}]'
```

## Debugging Commands

### Get Preview Server Logs and Status
```bash
# Logs (SSE stream; add --previous for a crashed container)
moat tasks logs <task_id> --server preview

# Per-server status
moat tasks status <task_id>
# raw proxy equivalent:
curl -s "http://localhost:8080/api/sandbox/v1/tasks/<task_id>/server/status" | jq '.servers'
```

### Test Health Check Manually
```bash
CONTAINER=$(docker ps --format "{{.Names}}" | grep moatless-sandbox | head -1)

# The sandbox container's own health endpoint
docker exec $CONTAINER curl -s http://localhost:3000/
```

### Test Bun Fetch vs Curl
```bash
# curl may prefer IPv6, Bun prefers IPv4
docker exec $CONTAINER curl -s -o /dev/null -w "%{http_code}" http://localhost:3000/
docker exec $CONTAINER bun -e "fetch('http://localhost:3000').then(r => console.log(r.status)).catch(e => console.log(e.message))"
```

## Health Check Flow

1. Sandbox container starts the dev server with the workspace's `startCommands`
2. Sandbox polls `http://localhost:{port}/` every second
3. After a successful response, marks the server as healthy
4. Backend polls the sandbox at `http://{SANDBOX_HOST}:{hostPort}/health`

If step 2 fails (IPv6 issue), step 4 will timeout.
