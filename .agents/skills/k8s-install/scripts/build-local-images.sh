#!/usr/bin/env bash
# Build the arm64 images that k8s/local/values-local.yaml expects.
# Usage: build-local-images.sh [backend|sandbox|t3]...   (default: all)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
T3CODE_DIR="${T3CODE_DIR:-$ROOT/../t3code}"
ORG="${REGISTRY_ORG:-aorwall}"
TAG="${IMAGE_TAG:-dev}"
RUST_VERSION="$(sed -nE 's/^channel = "([0-9.]+)"/\1/p' "$ROOT/rust-toolchain.toml")"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cd "$ROOT"

build_backend() {
  sed -E "s/rust:[0-9.]+-slim-bookworm/rust:${RUST_VERSION}-slim-bookworm/" \
    docker/Dockerfile.backend > "$WORK/Dockerfile.backend"
  docker buildx build --platform linux/arm64 --load \
    -t "$ORG/moatless-backend:$TAG" -f "$WORK/Dockerfile.backend" .
}

build_sandbox() {
  ./docker/build-sandbox.sh --local --tag "$TAG" --only sandbox-base
  docker buildx build --platform linux/arm64 \
    --build-arg "RUST_VERSION=$RUST_VERSION" \
    --output "type=local,dest=$WORK/binaries" \
    -f "$SCRIPT_DIR/Dockerfile.local-binaries" .
  docker build --platform linux/arm64 -t "$ORG/moatless-sandbox-tools:$TAG" \
    --build-context "binaries=$WORK/binaries" -f docker/Dockerfile.sandbox-tools .
  docker build --platform linux/arm64 -t "$ORG/moatless-sandbox:$TAG" \
    --build-arg "SANDBOX_BASE=$ORG/moatless-sandbox-base:$TAG" \
    --build-arg "SANDBOX_TOOLS=$ORG/moatless-sandbox-tools:$TAG" \
    --build-context "binaries=$WORK/binaries" -f docker/Dockerfile.sandbox .
}

build_t3() {
  docker build --platform linux/arm64 -t "$ORG/moatless-t3:$TAG" \
    --build-arg "CSP_FRAME_SRC=https: http://*.preview.moatless.localhost" \
    -f "$T3CODE_DIR/docker/Dockerfile" "$T3CODE_DIR"
}

[ $# -gt 0 ] || set -- backend sandbox t3
for target in "$@"; do
  case "$target" in
    backend | sandbox | t3) "build_$target" ;;
    *) echo "unknown target: $target" >&2; exit 2 ;;
  esac
done
