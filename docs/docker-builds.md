# Docker Build Guide

## Sandbox Image

Tasks run in one sandbox image. `docker/build-sandbox.sh` builds it and pushes
it under two tags that name the same image:

| Tag | Note |
|-----|------|
| `<org>/moatless-sandbox:<tag>` | The name to use |
| `<org>/moatless-sandbox-js:<tag>` | Alias for configurations that still name it |

`<org>` is `REGISTRY_ORG`, default `aorwall` (the published images). Export
`REGISTRY_ORG=<your-org>` to build and push under your own Docker Hub
organization.

The image is `moatless-sandbox-base` (`docker/Dockerfile.sandbox-base`) plus the
entrypoint scripts and the `sandbox` and `moat` binaries
(`docker/Dockerfile.sandbox`). The base holds the tooling: Node.js, npm, Bun,
pnpm, yarn, TypeScript/tsx, Vite+, Playwright with Chromium, the Claude Code and
Codex CLIs, Task, the Rust toolchain and git. Its header comment is the current
list.

## Building

```bash
# Build multi-arch and push both tags (the default)
./docker/build-sandbox.sh

# Build single-arch for the local Docker daemon, no push
./docker/build-sandbox.sh --local

# Build for K8s (linux/amd64) with a custom tag and push
./docker/build-sandbox.sh --platform linux/amd64 --tag v1.0.0 --push
```

## Build Options

| Option | Description |
|--------|-------------|
| `--tag TAG` | Image tag (default: `latest`) |
| `--platform ARCH` | Target platform (e.g., `linux/amd64`) |
| `--local` | Single-arch build for this host, no push |
| `--push` | Push after building |
| `--only TARGET` | Build only `sandbox-base` or `sandbox` |
| `--skip-base` | Reuse an existing `sandbox-base` |
| `--no-cache` | Build without Docker cache |
| `--help` | Show every option |

## Choosing the Image

The backend reads `SANDBOX_IMAGE` for Kubernetes sandboxes and
`DOCKER_SANDBOX_IMAGE` for Docker sandboxes; both default to
`aorwall/moatless-sandbox:latest`.

## Cross-Platform Builds

For K8s clusters (typically linux/amd64), always specify the platform:

```bash
./docker/build-sandbox.sh --platform linux/amd64 --push
```

This uses `docker buildx` for cross-platform builds.

## Publishing from CI

A push to `main` builds changed backend and sandbox images on native AMD64 and
ARM64 runners. Once both builds for an image succeed, CI publishes a shared
`:latest` tag and a `:sha-<short>` tag to Docker Hub under `REGISTRY_ORG`.

| Setting | Kind | Value |
|---|---|---|
| `DOCKERHUB_USERNAME`, `DOCKERHUB_TOKEN` | secrets of the `dockerhub` environment | a Docker Hub token with write access to `REGISTRY_ORG` |
| `REGISTRY_ORG` | variable, optional | the Docker Hub namespace; `aorwall` when unset |
| `CI_RUNNER`, `CI_RUNNER_LARGE` | variables, optional | runner labels for the CI jobs; `ubuntu-latest` when unset |
| `CI_RUNNER_ARM64` | variable, optional | native Linux ARM64 runner label; `ubuntu-24.04-arm` when unset |

A `workflow_dispatch` from another branch publishes nothing to Docker Hub. When
the `PRERELEASE_REGISTRY` variable is set, `publish-prerelease` pushes the
backend and sandbox images there instead, tagged `branch-<ref>` and
`sha-<short>`, through the OIDC role in `PRERELEASE_ROLE_ARN` (ECR only).

Restrict the `dockerhub` environment to `main`. A job that names a runner label
no runner serves stays queued for 24 hours and is then cancelled, with no error.
