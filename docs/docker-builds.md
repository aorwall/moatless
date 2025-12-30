# Docker Build Guide

## Worker Images

Two worker images are available for task execution in Kubernetes:

| Image | Size | Use Case |
|-------|------|----------|
| `aorwall/moatless-vibe-worker` | ~1.7GB | Full polyglot (Python, Node, Go, Rust, Java) |
| `aorwall/moatless-vibe-worker-js` | ~530MB | JavaScript/TypeScript only |

## Building Images

### Full Worker (all languages)

```bash
# Build for local platform
./docker/build-worker.sh

# Build for K8s (linux/amd64) and push
./docker/build-worker.sh --platform linux/amd64 --push

# Build with custom tag
./docker/build-worker.sh --tag v1.0.0 --push
```

### Slim JS/TS Worker

```bash
# Build for local platform
./docker/build-worker-js.sh

# Build for K8s and push
./docker/build-worker-js.sh --platform linux/amd64 --push
```

### Test Runner (for K8s integration tests)

```bash
./docker/build-test-runner.sh
```

## Build Options

All build scripts support:

| Option | Description |
|--------|-------------|
| `--tag TAG` | Custom image tag (default: `latest`) |
| `--platform ARCH` | Target platform (e.g., `linux/amd64`) |
| `--push` | Push to Docker Hub after building |
| `--no-cache` | Build without Docker cache |
| `--help` | Show help |

## Using Custom Worker Images

### In Kubernetes Settings

```json
{
  "kubernetes": {
    "image": "aorwall/moatless-vibe-worker-js:latest"
  }
}
```

### Via Environment Variable

```bash
K8S_WORKER_IMAGE=aorwall/moatless-vibe-worker-js:latest
```

## What's in Each Image

### Full Worker (`moatless-vibe-worker`)
- Node.js 22, Bun, npm/pnpm/yarn
- Python 3.12 (pyenv)
- Go 1.23
- Rust (rustup)
- Java 21, Maven
- Playwright + Chromium
- Claude Code CLI

### Slim Worker (`moatless-vibe-worker-js`)
- Node.js 22, Bun, npm/pnpm/yarn
- TypeScript, tsx
- Playwright (no browsers)
- Claude Code CLI

## Cross-Platform Builds

For K8s clusters (typically linux/amd64), always specify the platform:

```bash
./docker/build-worker.sh --platform linux/amd64 --push
```

This uses `docker buildx` for cross-platform builds.
