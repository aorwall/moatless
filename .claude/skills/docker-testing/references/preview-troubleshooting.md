# Preview Server Troubleshooting

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
# Get sandbox container
CONTAINER=$(docker ps --format "{{.Names}}" | grep moatless-moatable | head -1)
PORT=$(docker port $CONTAINER 8001 | cut -d: -f2)

# Check server logs - look for IPv6 address
curl -s "http://localhost:$PORT/server/logs" | jq -r '.logs'
# Look for: "Local: http://localhost:3000/" vs "http://[::1]:3000/"

# Test IPv4 (will fail)
docker exec $CONTAINER bun -e "fetch('http://localhost:3000').then(r => console.log(r.status)).catch(e => console.log('IPv4 failed:', e.message))"

# Test IPv6 (will succeed)
docker exec $CONTAINER bun -e "fetch('http://[::1]:3000').then(r => console.log(r.status)).catch(e => console.log('IPv6 failed:', e.message))"
```

**Fix**: Add `--host` flag to dev server command:
```bash
# Update repository config
curl -X PATCH "http://localhost:8000/api/v1/repositories/{repoId}" \
  -H "Content-Type: application/json" \
  -d '{"previewConfig": {"serverStartCommands": ["bun run dev -- --host"]}}'
```

### Timeout Issues

**Default timeout**: 90 seconds (configurable per repository)

**Diagnosis**:
```bash
# Check current timeout
curl -s "http://localhost:8000/api/v1/repositories/{repoId}" | jq '.previewConfig.startupTimeoutSeconds'
```

**Fix**: Increase timeout if `bun install` takes too long:
```bash
curl -X PATCH "http://localhost:8000/api/v1/repositories/{repoId}" \
  -H "Content-Type: application/json" \
  -d '{"previewConfig": {"startupTimeoutSeconds": 120}}'
```

## Debugging Commands

### Get Preview Server Logs
```bash
CONTAINER=$(docker ps --format "{{.Names}}" | grep moatless-moatable | head -1)
PORT=$(docker port $CONTAINER 8001 | cut -d: -f2)

# Full logs (shows bun install output, server startup)
curl -s "http://localhost:$PORT/server/logs" | jq -r '.logs'

# Server status
curl -s "http://localhost:$PORT/server/status" | jq .
```

### Test Health Check Manually
```bash
# From host (through port mapping)
curl -s "http://localhost:$PORT/health"

# Inside container
docker exec $CONTAINER curl -s http://localhost:3000/
```

### Test Bun Fetch vs Curl
```bash
# curl may prefer IPv6, Bun prefers IPv4
docker exec $CONTAINER curl -s -o /dev/null -w "%{http_code}" http://localhost:3000/
docker exec $CONTAINER bun -e "fetch('http://localhost:3000').then(r => console.log(r.status)).catch(e => console.log(e.message))"
```

## Health Check Flow

1. Sandbox container starts dev server with `serverStartCommands`
2. Sandbox polls `http://localhost:{port}/` every second
3. After successful response, marks server as healthy
4. Backend polls sandbox at `http://{SANDBOX_HOST}:{hostPort}/health`

If step 2 fails (IPv6 issue), step 4 will timeout.
