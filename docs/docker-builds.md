# Docker Build Guide

## Sandbox Images

Two sandbox images are available for task execution in Kubernetes:

| Image | Size | Use Case |
|-------|------|----------|
| `aorwall/moatless-vibe-sandbox` | ~1.7GB | Full polyglot (Python, Node, Go, Rust, Java) |
| `aorwall/moatless-vibe-sandbox-js` | ~530MB | JavaScript/TypeScript only |

## Building Images

### Full Sandbox (all languages)

```bash
# Build for local platform
./docker/build-sandbox.sh

# Build for K8s (linux/amd64) and push
./docker/build-sandbox.sh --platform linux/amd64 --push

# Build with custom tag
./docker/build-sandbox.sh --tag v1.0.0 --push
```

### Slim JS/TS Sandbox

```bash
# Build for local platform
./docker/build-sandbox.sh

# Build for K8s and push
./docker/build-sandbox.sh --platform linux/amd64 --push
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

## Using Custom Sandbox Images

### In Kubernetes Settings

```json
{
  "kubernetes": {
    "image": "aorwall/moatless-vibe-sandbox-js:latest"
  }
}
```

### Via Environment Variable

```bash
K8S_SANDBOX_IMAGE=aorwall/moatless-vibe-sandbox-js:latest
```

## What's in Each Image

### Full Sandbox (`moatless-vibe-sandbox`)
- Node.js 22, Bun, npm/pnpm/yarn
- Python 3.12 (pyenv)
- Go 1.23
- Rust (rustup)
- Java 21, Maven
- Playwright + Chromium
- Claude Code CLI

### Slim Sandbox (`moatless-vibe-sandbox-js`)
- Node.js 22, Bun, npm/pnpm/yarn
- TypeScript, tsx
- Playwright (no browsers)
- Claude Code CLI

## Cross-Platform Builds

For K8s clusters (typically linux/amd64), always specify the platform:

```bash
./docker/build-sandbox.sh --platform linux/amd64 --push
```

This uses `docker buildx` for cross-platform builds.
