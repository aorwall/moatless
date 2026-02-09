#!/usr/bin/env bash
set -euo pipefail

# Zig linker needs more file descriptors for codegen-units > 1
ulimit -n 10240 2>/dev/null || true

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IMAGE_NAME="${1:-aorwall/moatless-vibe-backend}"
IMAGE_TAG="${2:-latest}"

echo "==> Cross-compiling for x86_64-unknown-linux-gnu..."
cd "$SCRIPT_DIR"
cargo zigbuild --release --target x86_64-unknown-linux-gnu

# Resolve the workspace target directory (cargo puts binaries at workspace root)
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps 2>/dev/null | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')"
BINARY_DIR="${TARGET_DIR}/x86_64-unknown-linux-gnu/release"

echo "==> Building Docker image ${IMAGE_NAME}:${IMAGE_TAG}..."
docker buildx build \
    --platform linux/amd64 \
    --build-context "binary=${BINARY_DIR}" \
    -t "${IMAGE_NAME}:${IMAGE_TAG}" \
    --push \
    -f Dockerfile.cross \
    "$SCRIPT_DIR"

echo "==> Done: ${IMAGE_NAME}:${IMAGE_TAG}"
