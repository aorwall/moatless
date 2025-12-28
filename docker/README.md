# Moatless Vibe Docker Images

This directory contains Docker configurations for building two separate images:

## Images Overview

| Image | Base | Size | Purpose |
|-------|------|------|---------|
| `moatless-vibe` | `bun:alpine` | ~150MB | Main web application |
| `moatless-vibe-worker` | Ubuntu 24.04 | ~5-8GB | Worker pods for task execution |

## moatless-vibe (App Image)

Lightweight image for running the main web application server.

### Building

```bash
# Build with default settings
./docker/build-app.sh

# Build with custom tag
./docker/build-app.sh --tag v1.0.0

# Build for specific platform
./docker/build-app.sh --platform linux/amd64

# Build and push to registry
./docker/build-app.sh --push
```

### Docker directly

```bash
docker build -f docker/Dockerfile.app -t moatless-vibe:latest .
```

### Configuration

| Variable | Description | Default |
|----------|-------------|---------|
| `WORKER_MODE` | Must be `false` for app mode | `false` |
| `PORT` | API port | `8000` |
| `DATABASE_PROVIDER` | Database type | `sqlite` |
| `DATABASE_URL` | Database connection string | - |

## moatless-vibe-worker (Worker Image)

Full development environment for executing agent tasks. Includes Python, Rust, Go, Java, Node.js, Claude Code CLI, Playwright, and more.

### Building

```bash
# Build with default settings
./docker/build-worker.sh

# Build with custom tag
./docker/build-worker.sh --tag v1.0.0

# Build for specific platform
./docker/build-worker.sh --platform linux/amd64

# Build and push to registry
./docker/build-worker.sh --push
```

### Docker directly

```bash
docker build -f docker/Dockerfile.worker -t moatless-vibe-worker:latest .
```

### Configuration

| Variable | Description | Default |
|----------|-------------|---------|
| `WORKER_MODE` | Must be `true` for worker mode | `true` |
| `WORKER_API_URL` | Main backend API URL | Required |
| `WORKER_API_KEY` | API key for authentication | Required |
| `WORKER_TASK_ID` | Task ID for this worker | Required |
| `WORKSPACE_DIR` | Workspace directory | `/workspace` |
| `DATA_DIR` | Data storage directory | `/data` |
| `PORT` | Worker API port | `8001` |
| `ANTHROPIC_API_KEY` | Claude API key | Optional |

## Build Scripts

All build scripts support these options:

```
--tag TAG         Docker image tag (default: latest)
--platform ARCH   Build for specific platform (e.g., linux/amd64,linux/arm64)
--push            Push to Docker registry after building
--no-cache        Build without using Docker build cache
--help            Show help message
```

## Usage in Repository Configuration

To use the worker image in your repository configuration:

```json
{
  "codeExecution": {
    "workspaceType": "docker",
    "dockerImage": "moatless-vibe-worker:latest",
    "setupCommands": "npm install",
    "workerResources": {
      "cpuLimit": "2",
      "memoryLimit": "4g"
    }
  }
}
```

## Kubernetes Deployment

In Kubernetes, the Helm chart deploys both images:

- **Main deployment**: Uses `moatless-vibe` for the API/web app
- **Worker pods**: Uses `moatless-vibe-worker` for task execution

See `k8s/helm/moatless-vibe/values.yaml` for configuration.

## Entrypoints

### App Entrypoint (`docker-entrypoint-app.sh`)
- Ensures data directory exists
- Starts the bun server

### Worker Entrypoint (`docker-entrypoint.sh`)
- Validates `WORKER_MODE=true`
- Initializes firewall rules (if enabled)
- Configures git with GitHub token
- Runs setup commands
- Starts the worker server

## Troubleshooting

### Image not found
Build the image first:
```bash
./docker/build-app.sh     # for app image
./docker/build-worker.sh  # for worker image
```

### Connection refused (worker)
- Check that `WORKER_API_URL` is correct
- On Mac/Windows, use `host.docker.internal` instead of `localhost`
- Ensure the main backend is running

### Permission issues
- App image runs as `appuser` (UID 1000)
- Worker image runs as `ubuntu` (UID 1000)
- Ensure mounted directories have appropriate permissions
