#!/usr/bin/env bash
# Build the host-arch images that k8s/local/values-local.yaml expects.
# Usage: build-local-images.sh [backend|sandbox|t3]...   (default: all)
# K3D_CLUSTER=<name> also imports each built image into that k3d cluster.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
T3CODE_DIR="${T3CODE_DIR:-$ROOT/../t3code}"
ORG="${REGISTRY_ORG:-aorwall}"
TAG="${IMAGE_TAG:-dev}"
case "$(uname -m)" in
  x86_64 | amd64) HOST_PLATFORM=linux/amd64 ;;
  aarch64 | arm64) HOST_PLATFORM=linux/arm64 ;;
  *) echo "unsupported host architecture: $(uname -m)" >&2; exit 1 ;;
esac
PLATFORM="${PLATFORM:-$HOST_PLATFORM}"
# The `docker` driver writes straight into the image store. A `docker-container`
# builder left selected by `build-sandbox.sh --multi-arch` copies each image out
# on `--load` and keeps the DNS of the network it was created on.
export BUILDX_BUILDER="${BUILDX_BUILDER:-default}"
RUST_VERSION="$(sed -nE 's/^channel = "([0-9.]+)"/\1/p' "$ROOT/rust-toolchain.toml")"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cd "$ROOT"

build_backend() {
  sed -E "s/rust:[0-9.]+-slim-bookworm/rust:${RUST_VERSION}-slim-bookworm/" \
    docker/Dockerfile.backend > "$WORK/Dockerfile.backend"
  docker buildx build --platform "$PLATFORM" --load \
    -t "$ORG/moatless-backend:$TAG" -f "$WORK/Dockerfile.backend" .
  import_image "$ORG/moatless-backend:$TAG"
}

build_sandbox() {
  # `--platform` must follow `--local`, which resets it to the host arch.
  ./docker/build-sandbox.sh --local --platform "$PLATFORM" --tag "$TAG" --only sandbox-base
  docker buildx build --platform "$PLATFORM" \
    --build-arg "RUST_VERSION=$RUST_VERSION" \
    --output "type=local,dest=$WORK/binaries" \
    -f "$SCRIPT_DIR/Dockerfile.local-binaries" .
  docker build --platform "$PLATFORM" -t "$ORG/moatless-sandbox-tools:$TAG" \
    --build-context "binaries=$WORK/binaries" -f docker/Dockerfile.sandbox-tools .
  docker build --platform "$PLATFORM" -t "$ORG/moatless-sandbox:$TAG" \
    --build-arg "SANDBOX_BASE=$ORG/moatless-sandbox-base:$TAG" \
    --build-arg "SANDBOX_TOOLS=$ORG/moatless-sandbox-tools:$TAG" \
    --build-context "binaries=$WORK/binaries" -f docker/Dockerfile.sandbox .
  import_image "$ORG/moatless-sandbox:$TAG"
}

build_t3() {
  # Nothing pins a t3code version: the published image tracks its `main`.
  local behind
  if git -C "$T3CODE_DIR" fetch -q origin main 2>/dev/null; then
    behind="$(git -C "$T3CODE_DIR" rev-list --count HEAD..origin/main)"
    [ "$behind" = 0 ] || echo "warning: $T3CODE_DIR is $behind commits behind origin/main;" \
      "run: git -C $T3CODE_DIR pull --ff-only" >&2
  fi
  docker build --platform "$PLATFORM" -t "$ORG/moatless-t3:$TAG" \
    --build-arg "CSP_FRAME_SRC=https: http://*.preview.moatless.localhost" \
    -f "$T3CODE_DIR/docker/Dockerfile" "$T3CODE_DIR"
  import_image "$ORG/moatless-t3:$TAG"
}

# A k3d node runs its own containerd, so a pod cannot see the host's Docker
# store the way it can under Docker Desktop's kubeadm mode.
import_image() {
  [ -n "${K3D_CLUSTER:-}" ] || return 0
  k3d image import --cluster "$K3D_CLUSTER" "$1"
}

[ $# -gt 0 ] || set -- backend sandbox t3
for target in "$@"; do
  case "$target" in
    backend | sandbox | t3) "build_$target" ;;
    *) echo "unknown target: $target" >&2; exit 2 ;;
  esac
done
