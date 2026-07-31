# Moatless Vibe Docker Images

This directory contains Docker configurations for building the application and sandbox images.

## Images Overview

| Image | Base | Size | Purpose |
|-------|------|------|---------|
| `moatless-frontend` | `nginx:alpine` | ~150MB | Main web application (static frontend) |
| `moatless-sandbox` | Ubuntu 24.04 | ~500MB | Minimal base for sandboxes |
| `moatless-sandbox-js` | sandbox | ~2GB | JS/TS environment with Playwright + Rust toolchain |
| `moatless-sandbox-full` | sandbox | ~5-8GB | Full dev environment (Python, Go, Java; Rust from base) |

## Image Hierarchy

```
┌─────────────────────────────────┐
│     moatless-sandbox       │
│  Ubuntu 24.04, Node.js 22, npm  │
│  Claude Code CLI, git-delta     │
│  moatless user (UID 1000)       │
└───────────────┬─────────────────┘
                │
  ┌─────────────┴─────────────────────┐
  │                                   │
  ▼                                   ▼
┌─────────────────────────────┐    ┌─────────────────────────────┐
│      sandbox-js (default)   │    │     sandbox-full (full)     │
│  + Bun, pnpm, yarn          │    │  + Python 3.12 (pyenv)      │
│  + TypeScript               │    │  + Go 1.23                  │
│  + Rust (rustup) + protoc   │    │  + Java 21 + Maven          │
│  + Playwright + Chromium    │    │  (Rust from base)           │
│  + moatless-vibe app        │    │  + Bun, pnpm, yarn          │
└─────────────────────────────┘    │  + Playwright + Chromium    │
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
docker build -f docker/Dockerfile.app -t moatless-frontend:latest .
```

### Sandbox Images

Use the unified `build-sandbox.sh` script to build sandbox images in the correct order:

```bash
# Build all images (sandbox-base → sandbox; the latter is also tagged sandbox-js)
./docker/build-sandbox.sh

# Build with custom tag
./docker/build-sandbox.sh --tag v1.0.0

# Build and push to registry
./docker/build-sandbox.sh --push

# Build only specific target (assumes its dependencies exist in registry)
./docker/build-sandbox.sh --only sandbox-base
./docker/build-sandbox.sh --only sandbox

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
├── workspace/          # Persisted working area / agent cwd (WORKSPACE_DIR)
│   └── <repo-name>/    # Git repository checkout (repo dir)
├── data/               # Persistent data (DATA_DIR)
└── .claude/            # Claude Code config (CLAUDE_CONFIG_DIR)
    └── skills -> /usr/share/moatless-skills  # Symlinked at container startup
```

## moatless-frontend (App Image)

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
docker build -f docker/Dockerfile.app -t moatless-frontend:latest .
```

## moatless-sandbox-base (Stable Base)

Stable base image extended by the runtime sandbox images. Holds every layer
that doesn't depend on the Rust binaries, so cache hits are preserved across
normal code changes — only the binary `COPY` in the leaf images invalidates.

### Contents
- Ubuntu 24.04
- Node.js 22 + npm
- Claude Code CLI, Codex CLI
- Bun, pnpm, yarn, TypeScript, tsx
- Vite+ (`vp`) at `VP_HOME=/opt/vite-plus` — https://viteplus.dev/guide/
- Playwright + Chromium
- Rust toolchain (rustup/cargo/clippy/rustfmt) + protoc, pkg-config —
  enough to build the moatless-vibe Rust workspace in-sandbox
- Git, curl, wget, jq, ripgrep
- git-delta, GitHub CLI (gh), Docker CLI
- moatless user (UID 1000)

### Building

```bash
./docker/build-sandbox.sh --only sandbox-base
# or
docker build -f docker/Dockerfile.sandbox-base -t moatless-sandbox-base:latest .
```

## moatless-sandbox / moatless-sandbox-js (Runtime Sandbox)

The default runtime sandbox: `sandbox-base` plus entrypoint scripts and the
Rust `sandbox` and `moat` binaries. Published under two repository names that
point at the same image content:

- `aorwall/moatless-sandbox:<tag>`
- `aorwall/moatless-sandbox-js:<tag>` (alias kept for back-compat)

### Building

```bash
./docker/build-sandbox.sh --only sandbox
# or
docker build -f docker/Dockerfile.sandbox \
  --build-arg SANDBOX_BASE=moatless-sandbox-base:latest \
  --build-context binaries=/path/to/staged/binaries \
  -t moatless-sandbox:latest \
  -t moatless-sandbox-js:latest .
```

## moatless-sandbox-full (Full Sandbox)

Full development environment for executing agent tasks with all major languages.

### Contents (in addition to sandbox-base)
- Python 3.12 (via pyenv)
- Go 1.23
- Java 21 + Maven

(Rust comes from sandbox-base and is available in every sandbox variant.)

### Building

```bash
docker build -f docker/Dockerfile.sandbox-full \
  --build-arg BASE_IMAGE=moatless-sandbox-base:latest \
  --build-context binaries=/path/to/staged/binaries \
  -t moatless-sandbox-full:latest .
```

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `WORKSPACE_DIR` | Workspace directory (agent cwd; repo cloned into `WORKSPACE_DIR/<repo-name>`) | `/opt/moatless/workspace` |
| `DATA_DIR` | Data storage directory | `/opt/moatless/data` |
| `CLAUDE_CONFIG_DIR` | Claude config directory | `/opt/moatless/.claude` |
| `PORT` | Sandbox API port | `8001` |
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

- **Main deployment**: Uses `moatless-frontend` for the web app
- **Worker pods**: Uses `moatless-sandbox-js` (or configured alternative) for task execution

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
    "dockerImage": "moatless-sandbox-js:latest",
    "setupCommands": "npm install",
    "workerResources": {
      "cpuLimit": "2",
      "memoryLimit": "4g"
    }
  }
}
```

Use `moatless-sandbox-full:latest` for projects needing Python, Rust, Go, or Java.

## Troubleshooting

### Image not found
Build the images first:
```bash
./docker/build-app.sh       # for app image
./docker/build-sandbox.sh   # for sandbox images
```

### Connection refused (sandbox)
- On Mac/Windows, use `host.docker.internal` instead of `localhost`
- Ensure the main backend is running

### Permission issues
- All sandbox images run as `moatless` (UID 1000)
- In Kubernetes, `fsGroup: 1000` handles PVC permissions
- For local Docker, mount volumes with correct ownership or use `--user 1000:1000`
