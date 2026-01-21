# Moatless Vibe Docker Images

This directory contains Docker configurations for building three image types:

## Images Overview

| Image | Base | Size | Purpose |
|-------|------|------|---------|
| `moatless-vibe` | `bun:alpine` | ~150MB | Main web application |
| `moatless-vibe-worker-base` | Ubuntu 24.04 | ~500MB | Minimal base for workers |
| `moatless-vibe-worker` | worker-base | ~5-8GB | Full dev environment (Python, Rust, Go, Java) |
| `moatless-vibe-worker-js` | worker-base | ~2GB | JS/TS environment with Playwright |

## Image Hierarchy

```
                    ┌─────────────────────────────────┐
                    │        worker-base              │
                    │  Ubuntu 24.04, Node.js 22, npm  │
                    │  Claude Code CLI, git-delta     │
                    │  moatless user (UID 1000)       │
                    └───────────────┬─────────────────┘
                                    │
              ┌─────────────────────┴─────────────────────┐
              │                                           │
              ▼                                           ▼
┌─────────────────────────────┐           ┌─────────────────────────────┐
│        worker (full)        │           │      worker-js (JS-only)    │
│  + Python 3.12 (pyenv)      │           │  + Bun, pnpm, yarn          │
│  + Rust (rustup)            │           │  + TypeScript               │
│  + Go 1.23                  │           │  + Playwright + Chromium    │
│  + Java 21 + Maven          │           │  + moatless-vibe app        │
│  + Bun, pnpm, yarn          │           └─────────────────────────────┘
│  + Playwright + Chromium    │
│  + moatless-vibe app        │
└─────────────────────────────┘
```

### Build Script

The unified `build-worker.sh` script builds all images in the correct order:

```bash
# Build all images (base → worker → worker-js)
./docker/build-worker.sh

# Build with custom tag
./docker/build-worker.sh --tag v1.0.0

# Build and push to registry
./docker/build-worker.sh --push

# Build only specific image (assumes base exists)
./docker/build-worker.sh --only worker-js

# Skip base rebuild (use cached)
./docker/build-worker.sh --skip-base
```

Docker layer caching means unchanged layers won't be rebuilt.

## User Configuration

All worker images run as the `moatless` user:
- **Username**: `moatless`
- **UID/GID**: 1000
- **Home directory**: `/opt/moatless`
- **No sudo access**

### Directory Structure
```
/opt/moatless/          # Root directory (PVC mount in K8s, volume mount in Docker)
├── repo/               # Git repository (WORKSPACE_DIR)
├── data/               # Persistent data (DATA_DIR)
└── .claude/            # Claude Code config (CLAUDE_CONFIG_DIR)
    └── skills -> /usr/share/moatless-skills  # Symlinked at container startup
```

## moatless-vibe (App Image)

Lightweight image for running the main web application server.

### Building

```bash
# Build with default settings
./docker/build-app.sh

# Build with custom tag
./docker/build-app.sh --tag v1.0.0

# Build and push to registry
./docker/build-app.sh --push
```

### Docker directly

```bash
docker build -f docker/Dockerfile.app -t moatless-vibe:latest .
```

## moatless-vibe-worker-base (Minimal Base)

Minimal base image with only essential tools. Used as the base for the full worker image.

### Contents
- Ubuntu 24.04
- Node.js 22 + npm
- Claude Code CLI
- Git, curl, wget, jq, ripgrep
- git-delta

### Building

```bash
# Via unified script
./docker/build-worker.sh --only base

# Or directly
docker build -f docker/Dockerfile.worker-base -t ghcr.io/aorwall/moatless-vibe-worker-base:latest .
```

## moatless-vibe-worker (Full Worker)

Full development environment for executing agent tasks. Includes all major programming languages.

### Contents (in addition to base)
- Python 3.12 (via pyenv)
- Rust (via rustup)
- Go 1.23
- Java 21 + Maven
- Bun, pnpm, yarn
- Playwright + Chromium

### Building

```bash
# Build with default settings
./docker/build-worker.sh

# Build with custom tag
./docker/build-worker.sh --tag v1.0.0

# Build and push to registry
./docker/build-worker.sh --push
```

### Docker directly

```bash
# Note: requires worker-base to be built first
docker build -f docker/Dockerfile.worker -t moatless-vibe-worker:latest .
```

## moatless-vibe-worker-js (JS-Only Worker)

JavaScript/TypeScript-focused worker image with Playwright for browser automation. Based on `worker-base`.

### Contents (in addition to base)
- Bun, pnpm, yarn
- TypeScript
- Playwright + Chromium

### Building

```bash
# Via unified script
./docker/build-worker.sh --only worker-js

# Or directly (requires worker-base to be built first)
docker build -f docker/Dockerfile.worker-js -t ghcr.io/aorwall/moatless-vibe-worker-js:latest .
```

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `WORKER_MODE` | Must be `true` for worker mode | `true` |
| `WORKER_API_URL` | Main backend API URL | Required |
| `WORKER_AUTH_TOKEN` | JWT token for authentication | Required |
| `WORKSPACE_DIR` | Workspace directory | `/opt/moatless/repo` |
| `DATA_DIR` | Data storage directory | `/opt/moatless/data` |
| `CLAUDE_CONFIG_DIR` | Claude config directory | `/opt/moatless/.claude` |
| `PORT` | Worker API port | `8001` |
| `ANTHROPIC_API_KEY` | Claude API key | Optional |

## Build Scripts

The `build-worker.sh` script supports these options:

```
--tag TAG         Docker image tag (default: latest)
--platform ARCH   Build for specific platform (e.g., linux/amd64)
--push            Push to Docker registry after building
--no-cache        Build without using Docker build cache
--only TARGET     Build only one target: base, worker, or worker-js
--skip-base       Skip building base (assumes it exists)
--help            Show help message
```

The `build-app.sh` script builds only the main application image.

## Kubernetes Deployment

In Kubernetes, the Helm chart deploys:

- **Main deployment**: Uses `moatless-vibe` for the API/web app
- **Worker pods**: Uses `moatless-vibe-worker` (or configured alternative) for task execution

### Permissions

The Helm chart uses `fsGroup: 1000` to ensure mounted PVCs are writable by the `moatless` user (GID 1000). The container runs as whatever user is defined in the Dockerfile.

### PVC Mount

Worker pods mount the PVC at `/opt/moatless` (the user's home directory), providing persistent storage for:
- Cloned repositories
- Build artifacts
- Claude Code configuration

See `k8s/helm/moatless-vibe/values.yaml` for configuration.

## Usage in Repository Configuration

To use a specific worker image in your repository configuration:

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

Use `moatless-vibe-worker-js:latest` for JavaScript/TypeScript-only projects.

## Troubleshooting

### Image not found
Build the images first:
```bash
./docker/build-app.sh     # for app image
./docker/build-worker.sh  # for all worker images (base, worker, worker-js)
```

### Connection refused (worker)
- Check that `WORKER_API_URL` is correct
- On Mac/Windows, use `host.docker.internal` instead of `localhost`
- Ensure the main backend is running

### Permission issues
- All worker images run as `moatless` (UID 1000)
- In Kubernetes, `fsGroup: 1000` handles PVC permissions
- For local Docker, mount volumes with correct ownership or use `--user 1000:1000`
