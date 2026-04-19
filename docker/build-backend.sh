#!/usr/bin/env bash
set -euo pipefail

# Build and push the backend Docker image using local cross-compilation.
#
# Uses cargo-zigbuild to cross-compile, then packages the binary into
# a minimal distroless image via Dockerfile.backend-cross.

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Zig linker needs more file descriptors for codegen-units > 1
ulimit -n 10240 2>/dev/null || true

# Change to repository root (like all other build scripts)
cd "$(dirname "$0")/.."

IMAGE_NAME="${1:-aorwall/moatless-vibe-backend}"
IMAGE_TAG="${2:-latest}"

# Opt-in registry-backed buildcache. Set USE_REGISTRY_CACHE=1 to enable.
CACHE_ARGS=()
if [ "${USE_REGISTRY_CACHE:-0}" = "1" ]; then
    CACHE_ARGS+=(--cache-from "type=registry,ref=${IMAGE_NAME}:buildcache")
fi

echo -e "${YELLOW}==> Cross-compiling for x86_64-unknown-linux-gnu (glibc 2.36)...${NC}"
cargo zigbuild -p moatless-vibe-backend --release --target x86_64-unknown-linux-gnu.2.36

# Resolve the workspace target directory (cargo puts binaries at workspace root)
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps 2>/dev/null | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')"
BINARY_DIR="${TARGET_DIR}/x86_64-unknown-linux-gnu/release"

echo -e "${YELLOW}==> Building Docker image ${IMAGE_NAME}:${IMAGE_TAG}...${NC}"
docker buildx build \
    --platform linux/amd64 \
    --build-context "binary=${BINARY_DIR}" \
    "${CACHE_ARGS[@]}" \
    -t "${IMAGE_NAME}:${IMAGE_TAG}" \
    --push \
    -f docker/Dockerfile.backend-cross \
    .

echo -e "${GREEN}==> Done: ${IMAGE_NAME}:${IMAGE_TAG}${NC}"
