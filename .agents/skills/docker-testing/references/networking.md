# Docker Networking

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────┐
│ Docker Network (docker-compose_moatless)                       │
│                                                               │
│  ┌──────────────────┐     ┌──────────────────────────────┐   │
│  │ moatless-backend │     │ moatless-sandbox-{taskId}    │   │
│  │                  │     │                              │   │
│  │ HTTP  8080       │◀───▶│ gRPC 50052                   │   │
│  │ gRPC  50051      │     │ HTTP 8001                    │   │
│  │ Metrics 9090     │     │ ┌────────────────────────┐   │   │
│  │                  │     │ │ Dev Server (Vite/etc)  │   │   │
│  │                  │     │ │ Port 3000 (internal)   │   │   │
│  │                  │     │ └────────────────────────┘   │   │
│  └──────────────────┘     └──────────────────────────────┘   │
│                                                               │
│  ┌──────────────────┐     ┌──────────────────────────────┐   │
│  │ moatless-redis   │     │ moatless-postgres            │   │
│  │ Port 6379        │     │ Port 5432                    │   │
│  └──────────────────┘     └──────────────────────────────┘   │
└──────────────────────────────────────────────────────────────┘
```

## Container Communication

Backend and sandbox containers communicate via the shared Docker network (`docker-compose_moatless`). The backend resolves sandbox containers by their deterministic container name.

### Configuration

**docker-compose.yml**:
```yaml
backend:
  environment:
    SANDBOX_PROVIDER: docker
    DOCKER_NETWORK: docker-compose_moatless
    DOCKER_BACKEND_HOSTNAME: moatless-backend
```

**Code** (`backend/src/sandbox/docker/provider.rs`):
```rust
// Backend reaches sandbox via container name on Docker network:
// http://moatless-sandbox-{taskId}:{grpc_port}
```

## Container Communication Patterns

### Backend → Sandbox (gRPC)
```
Backend container (moatless-backend)
    │
    ▼ http://moatless-sandbox-{taskId}:50052  (Docker network)
    │
Sandbox container gRPC port 50052
```

### Sandbox → Backend (gRPC)
```
Sandbox container
    │
    ▼ http://moatless-backend:50051  (Docker network, via DOCKER_BACKEND_HOSTNAME)
    │
Backend gRPC server port 50051
```

### Backend → Redis/Postgres
Uses Docker network directly:
```yaml
backend:
  depends_on:
    - redis
    - postgres
  environment:
    REDIS_URL: redis://redis:6379
    DATABASE_URL: postgresql://moatless:moatless@postgres:5432/moatless
```

## Port Mappings

### Static (docker-compose)
| Service | Container Port | Host Port |
|---------|---------------|-----------|
| Backend | 8080 | 8080 |
| Redis | 6379 | 6379 |
| Postgres | 5432 | 5432 |

### Dynamic (sandbox containers)
Sandbox containers get random host ports:
```bash
# Find sandbox port
docker port moatless-sandbox-abc12345 8001
# Output: 0.0.0.0:58164
```

## host.docker.internal

Special DNS name that resolves to the Docker host from inside containers.

**Docker Desktop (Mac/Windows)**: Works automatically

**Linux**: Requires explicit mapping:
```yaml
services:
  backend:
    extra_hosts:
      - "host.docker.internal:host-gateway"
```

Already configured in `docker-compose.yml`.

## Troubleshooting

### Backend Can't Reach Sandbox
**Symptom**: Timeout when starting task

**Check**:
```bash
# Verify sandbox container is on the moatless network
docker network inspect docker-compose_moatless --format '{{range .Containers}}{{.Name}} {{end}}' | tr ' ' '\n' | grep moatless-sandbox

# Check sandbox container exists and has port mapping
docker ps --format "table {{.Names}}\t{{.Ports}}" | grep moatless-sandbox
```

### Sandbox Container Not Accessible
```bash
# Get the mapped port (for host-side debugging)
PORT=$(docker port moatless-sandbox-{taskId} 8001 | cut -d: -f2)

# Test from host (via port mapping)
curl http://localhost:$PORT/health

# Test via Docker network (simulates what backend does)
docker exec moatless-backend wget -qO- http://moatless-sandbox-{taskId}:8001/health
```
