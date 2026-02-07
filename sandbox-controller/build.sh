#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IMAGE_NAME="${1:-aorwall/sandbox-controller}"
IMAGE_TAG="${2:-latest}"

echo "==> Cross-compiling for x86_64-unknown-linux-gnu..."
cd "$SCRIPT_DIR"
cargo zigbuild --release --target x86_64-unknown-linux-gnu

echo "==> Building Docker image ${IMAGE_NAME}:${IMAGE_TAG}..."
docker buildx build \
    --platform linux/amd64 \
    --build-context binary=target/x86_64-unknown-linux-gnu/release \
    -t "${IMAGE_NAME}:${IMAGE_TAG}" \
    --push \
    -f Dockerfile.cross \
    "$SCRIPT_DIR"

echo "==> Done: ${IMAGE_NAME}:${IMAGE_TAG}"
