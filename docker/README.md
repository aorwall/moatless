# Moatless Vibe Docker Images

This directory contains Docker configurations for building the application and sandbox images.

## Images Overview

| Image | Base | Size | Purpose |
|-------|------|------|---------|
| `moatless-vibe` | `bun:alpine` | ~150MB | Main web application (self-contained) |
| `moatless-vibe-sandbox` | Ubuntu 24.04 | ~500MB | Minimal base for sandboxes |
| `moatless-vibe-sandbox-js` | sandbox | ~2GB | JS/TS environment with Playwright |
| `moatless-vibe-sandbox-full` | sandbox | ~5-8GB | Full dev environment (Python, Rust, Go, Java) |

## Image Hierarchy

```
┌─────────────────────────────────┐
│     moatless-vibe-sandbox       │
│  Ubuntu 24.04, Node.js 22, npm  │
│  Claude Code CLI, git-delta     │
│  moatless user (UID 1000)       │
└───────────────┬─────────────────┘
                │
  ┌─────────────┴─────────────────────┐
  │                                   │
  ▼                                   ▼
┌─────────────────────────────┐    ┌─────────────────────────────┐
│      sandbox-js (JS-only)   │    │     sandbox-full (full)     │
│  + Bun, pnpm, yarn          │    │  + Python 3.12 (pyenv)      │
│  + TypeScript               │    │  + Rust (rustup)            │
│  + Playwright + Chromium    │    │  + Go 1.23                  │
│  + moatless-vibe app        │    │  + Java 21 + Maven          │
└─────────────────────────────┘    │  + Bun, pnpm, yarn          │
                                   │  + Playwright + Chromium    │
                                   │  + moatless-vibe app        │
                                   └─────────────────────────────┘
```

## Building Images

### Application Image

The app image is self-contained (no separate base image needed):

```bash
# Build with default settings
./docker/build-app.sh

# Build with custom tag
./docker/build-app.sh --tag v1.0.0

# Build and push to registry
./docker/build-app.sh --push

# Build directly with Docker
docker build -f docker/Dockerfile.app -t moatless-vibe:latest .
```

### Sandbox Images

Use the unified `build-sandbox.sh` script to build sandbox images in the correct order:

```bash
# Build all images (sandbox → sandbox-js)
./docker/build-sandbox.sh

# Build with custom tag
./docker/build-sandbox.sh --tag v1.0.0

# Build and push to registry
./docker/build-sandbox.sh --push

# Build only specific image (assumes base exists)
./docker/build-sandbox.sh --only sandbox-js

# Skip base rebuild (use cached)
./docker/build-sandbox.sh --skip-base

# Force rebuild all layers
./docker/build-sandbox.sh --no-cache
```

Docker layer caching means unchanged layers won't be rebuilt.

## User Configuration

All sandbox images run as the `moatless` user:
- **Username**: `moatless`
- **UID/GID**: 1000
- **Home directory**: `/home/moatless`
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

### Contents
- Alpine Linux with Bun runtime
- Git, Docker CLI
- tini (init system)
- Pre-built application

### Building

```bash
./docker/build-app.sh
# or
docker build -f docker/Dockerfile.app -t moatless-vibe:latest .
```

## moatless-vibe-sandbox (Base Sandbox)

Minimal base image with essential tools. Used as the base for sandbox-js and sandbox-full.

### Contents
- Ubuntu 24.04
- Node.js 22 + npm
- Claude Code CLI
- Git, curl, wget, jq, ripgrep
- git-delta, GitHub CLI (gh)

### Building

```bash
./docker/build-sandbox.sh --only base
# or
docker build -f docker/Dockerfile.sandbox -t moatless-vibe-sandbox:latest .
```

## moatless-vibe-sandbox-js (JS/TS Sandbox)

JavaScript/TypeScript-focused sandbox with Playwright for browser automation.

### Contents (in addition to base)
- Bun, pnpm, yarn
- TypeScript
- Playwright + Chromium

### Building

```bash
./docker/build-sandbox.sh --only sandbox-js
# or (requires sandbox to be built first)
docker build -f docker/Dockerfile.sandbox-js \
  --build-arg BASE_IMAGE=moatless-vibe-sandbox:latest \
  -t moatless-vibe-sandbox-js:latest .
```

## moatless-vibe-sandbox-full (Full Sandbox)

Full development environment for executing agent tasks with all major languages.

### Contents (in addition to base)
- Python 3.12 (via pyenv)
- Rust (via rustup)
- Go 1.23
- Java 21 + Maven
- Bun, pnpm, yarn
- Playwright + Chromium

### Building

```bash
docker build -f docker/Dockerfile.sandbox-full \
  --build-arg BASE_IMAGE=moatless-vibe-sandbox:latest \
  -t moatless-vibe-sandbox-full:latest .
```

## Environment Variables

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

## Build Script Options

### build-sandbox.sh

```
--tag TAG         Docker image tag (default: latest)
--platform ARCH   Build for specific platform (e.g., linux/amd64)
--multi-arch      Build for both amd64 and arm64 (requires --push)
--local           Build single-arch locally (no push)
--push            Push to Docker registry after building
--no-cache        Build without using Docker build cache
--only TARGET     Build only one target: base or sandbox-js
--skip-base       Skip building base (assumes it exists)
--help            Show help message
```

### build-app.sh

```
--tag TAG         Docker image tag (default: latest)
--platform ARCH   Build for specific platform
--push            Push to Docker registry after building
--help            Show help message
```

## Kubernetes Deployment

In Kubernetes, the Helm chart deploys:

- **Main deployment**: Uses `moatless-vibe` for the API/web app
- **Worker pods**: Uses `moatless-vibe-sandbox-js` (or configured alternative) for task execution

### Permissions

The Helm chart uses `fsGroup: 1000` to ensure mounted PVCs are writable by the `moatless` user (GID 1000).

### PVC Mount

Worker pods mount the PVC at `/opt/moatless`, providing persistent storage for:
- Cloned repositories
- Build artifacts
- Claude Code configuration

See `k8s/helm/moatless-vibe/values.yaml` for configuration.

## Usage in Repository Configuration

To use a specific sandbox image in your repository configuration:

```json
{
  "codeExecution": {
    "workspaceType": "docker",
    "dockerImage": "moatless-vibe-sandbox-js:latest",
    "setupCommands": "npm install",
    "workerResources": {
      "cpuLimit": "2",
      "memoryLimit": "4g"
    }
  }
}
```

Use `moatless-vibe-sandbox-full:latest` for projects needing Python, Rust, Go, or Java.

## Troubleshooting

### Image not found
Build the images first:
```bash
./docker/build-app.sh       # for app image
./docker/build-sandbox.sh   # for sandbox images
```

### Connection refused (worker)
- Check that `WORKER_API_URL` is correct
- On Mac/Windows, use `host.docker.internal` instead of `localhost`
- Ensure the main backend is running

### Permission issues
- All sandbox images run as `moatless` (UID 1000)
- In Kubernetes, `fsGroup: 1000` handles PVC permissions
- For local Docker, mount volumes with correct ownership or use `--user 1000:1000`
