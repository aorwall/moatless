#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default values
# Registry org segment. Override via REGISTRY_ORG env var; defaults to `aorwall`
# so upstream builds work unchanged. Forks export REGISTRY_ORG=<their-org>.
REGISTRY="${REGISTRY_ORG:-aorwall}"
TAG="latest"
PLATFORM="linux/amd64,linux/arm64"
MULTI_ARCH=true
PUSH=true
USE_CACHE=true
USE_REGISTRY_CACHE=false
BUILD_SANDBOX_BASE=true
BUILD_SANDBOX=true

# Sandbox is published under two tags pointing at the same image:
#   - ${REGISTRY}/moatless-sandbox:<tag>
#   - ${REGISTRY}/moatless-sandbox-js:<tag>  (alias kept for back-compat
#     with downstream consumers that still reference sandbox-js).
SANDBOX_IMAGE_NAME="moatless-sandbox"
SANDBOX_JS_ALIAS_NAME="moatless-sandbox-js"

# CLI versions — single source of truth in docker/cli-versions.env. An env var
# set before invoking this script still wins (e.g. CLAUDE_CLI_VERSION=x ./build).
CLI_VERSIONS_FILE="$(cd "$(dirname "$0")" && pwd)/cli-versions.env"
_default_claude="$(sed -n 's/^CLAUDE_CLI_VERSION=//p' "$CLI_VERSIONS_FILE")"
_default_codex="$(sed -n 's/^CODEX_CLI_VERSION=//p' "$CLI_VERSIONS_FILE")"
CLAUDE_CLI_VERSION="${CLAUDE_CLI_VERSION:-$_default_claude}"
CODEX_CLI_VERSION="${CODEX_CLI_VERSION:-$_default_codex}"

# Parse command line arguments
while [[ $# -gt 0 ]]; do
  case $1 in
    --tag)
      TAG="$2"
      shift 2
      ;;
    --platform)
      PLATFORM="$2"
      shift 2
      ;;
    --push)
      PUSH=true
      shift
      ;;
    --multi-arch)
      MULTI_ARCH=true
      PLATFORM="linux/amd64,linux/arm64"
      shift
      ;;
    --local)
      MULTI_ARCH=false
      PUSH=false
      case "$(uname -m)" in
        x86_64|amd64)  PLATFORM="linux/amd64" ;;
        aarch64|arm64) PLATFORM="linux/arm64" ;;
        *) echo -e "${RED}Unsupported host architecture: $(uname -m)${NC}"; exit 1 ;;
      esac
      shift
      ;;
    --no-cache)
      USE_CACHE=false
      shift
      ;;
    --use-registry-cache)
      USE_REGISTRY_CACHE=true
      shift
      ;;
    --only)
      case $2 in
        sandbox-base|base)
          BUILD_SANDBOX=false
          ;;
        sandbox|sandbox-js)
          # sandbox-js is just an alias tag for sandbox; same build target.
          BUILD_SANDBOX_BASE=false
          ;;
        *)
          echo -e "${RED}Unknown target: $2. Use: sandbox-base, sandbox${NC}"
          exit 1
          ;;
      esac
      shift 2
      ;;
    --skip-base)
      BUILD_SANDBOX_BASE=false
      shift
      ;;
    --help)
      echo "Usage: $0 [OPTIONS]"
      echo ""
      echo "Build moatless-vibe sandbox Docker images."
      echo ""
      echo "Images built (in order):"
      echo "  1. sandbox-base - Stable base (Ubuntu, Node, Bun, pnpm, Playwright, CLIs)"
      echo "  2. sandbox      - sandbox-base + entrypoint scripts + Rust binaries"
      echo "                    Pushed under two tags: sandbox:<tag> and sandbox-js:<tag>"
      echo ""
      echo "Options:"
      echo "  --tag TAG         Docker image tag (default: latest)"
      echo "  --platform ARCH   Build for specific platform (e.g., linux/amd64)"
      echo "  --multi-arch      Build for both amd64 and arm64 (requires --push)"
      echo "  --local           Build single-arch locally (no push)"
      echo "  --push            Push to Docker registry after building"
      echo "  --no-cache        Build without using Docker build cache"
      echo "  --use-registry-cache  Pull/push buildcache from registry (opt-in)"
      echo "  --only TARGET     Build only one target: sandbox-base or sandbox"
      echo "  --skip-base       Skip building sandbox-base (assumes it exists)"
      echo "  --help            Show this help message"
      echo ""
      echo "Examples:"
      echo "  $0                        # Build multi-arch and push to Docker Hub (default)"
      echo "  $0 --local                # Build single-arch locally (no push)"
      echo "  $0 --only sandbox         # Build only sandbox (assumes sandbox-base exists)"
      echo "  $0 --skip-base            # Skip sandbox-base; rebuild sandbox only"
      echo "  $0 --no-cache             # Force rebuild all layers (use after code changes)"
      exit 0
      ;;
    *)
      echo -e "${RED}Unknown option: $1${NC}"
      exit 1
      ;;
  esac
done

# Change to repository root
cd "$(dirname "$0")/.."

# Validate multi-arch requires push
if [ "$MULTI_ARCH" = true ] && [ "$PUSH" = false ]; then
  echo -e "${RED}Error: --multi-arch requires --push (multi-arch images can't be loaded locally)${NC}"
  exit 1
fi

# Set up buildx builder for multi-arch builds
if [ "$MULTI_ARCH" = true ]; then
  BUILDER_NAME="moatless-multiarch"
  if ! docker buildx inspect "$BUILDER_NAME" > /dev/null 2>&1; then
    echo -e "${YELLOW}Creating buildx builder for multi-arch builds...${NC}"
    docker buildx create --name "$BUILDER_NAME" --driver docker-container --bootstrap
  fi
  docker buildx use "$BUILDER_NAME"
  echo -e "${GREEN}✓ Using buildx builder: $BUILDER_NAME${NC}"
  echo ""
fi

# Cleanup function for staged files
BINARIES_DIR=""
cleanup() {
  if [ -n "$BINARIES_DIR" ] && [ -d "$BINARIES_DIR" ]; then
    echo -e "${YELLOW}Cleaning up staged binaries...${NC}"
    rm -rf "$BINARIES_DIR"
  fi
}
trap cleanup EXIT

# Cross-compile Rust sandbox binary for target architectures.
# Needed by sandbox (the only leaf image that COPYs binaries here).
if [ "$BUILD_SANDBOX" = true ]; then
  if ! command -v cargo-zigbuild &> /dev/null; then
    echo -e "${RED}Error: cargo-zigbuild required for cross-compilation.${NC}"
    echo -e "${RED}Install: cargo install cargo-zigbuild && pip3 install ziglang${NC}"
    exit 1
  fi

  # Zig linker needs more file descriptors
  ulimit -n 10240 2>/dev/null || true

  # Resolve workspace target dir (cargo puts binaries at workspace root)
  TARGET_DIR="$(cargo metadata --format-version 1 --no-deps 2>/dev/null | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')"

  # Determine which architectures to build
  TARGETS=()
  if echo "$PLATFORM" | grep -q "amd64"; then
    TARGETS+=("x86_64-unknown-linux-gnu")
  fi
  if echo "$PLATFORM" | grep -q "arm64"; then
    TARGETS+=("aarch64-unknown-linux-gnu")
  fi

  # Stage binaries into a temp dir: amd64/sandbox, arm64/sandbox
  BINARIES_DIR="$(mktemp -d)"
  mkdir -p "$BINARIES_DIR/amd64" "$BINARIES_DIR/arm64"

  for TARGET in "${TARGETS[@]}"; do
    echo -e "${YELLOW}Cross-compiling sandbox + moat CLI for ${TARGET}...${NC}"
    cargo zigbuild -p moatless-sandbox -p moatless-cli --release --target "$TARGET"
    if [ $? -ne 0 ]; then
      echo -e "${RED}✗ Rust build failed for ${TARGET}!${NC}"
      exit 1
    fi

    # Map Rust target triple to Docker TARGETARCH
    case "$TARGET" in
      x86_64-*)  ARCH_DIR="amd64" ;;
      aarch64-*) ARCH_DIR="arm64" ;;
    esac
    cp "${TARGET_DIR}/${TARGET}/release/sandbox" "$BINARIES_DIR/$ARCH_DIR/sandbox"
    cp "${TARGET_DIR}/${TARGET}/release/moat" "$BINARIES_DIR/$ARCH_DIR/moat"
    echo -e "${GREEN}✓ sandbox + moat binaries staged for ${ARCH_DIR}${NC}"
  done
  echo ""
fi

# Helper function to build an image. Accepts an optional 4th argument: a
# space-separated list of *additional* image names to tag (same registry, same
# TAG) alongside the primary one — used to publish sandbox under both
# moatless-sandbox and moatless-sandbox-js without rebuilding.
build_image() {
  local dockerfile=$1
  local image_name=$2
  local extra_args="${3:-}"
  local extra_tag_names="${4:-}"
  local full_image="${REGISTRY}/${image_name}:${TAG}"

  echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
  echo -e "${GREEN}Building ${image_name}...${NC}"
  echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

  # Use buildx for multi-arch, regular docker build otherwise
  if [ "$MULTI_ARCH" = true ]; then
    BUILD_CMD="docker buildx build --platform $PLATFORM --push -t ${full_image}"
  else
    BUILD_CMD="docker build -t ${full_image}"
    if [ -n "$PLATFORM" ]; then
      BUILD_CMD="$BUILD_CMD --platform $PLATFORM"
    fi
  fi

  # Add extra alias tags (same content, different repository name).
  if [ -n "$extra_tag_names" ]; then
    for alias_name in $extra_tag_names; do
      BUILD_CMD="$BUILD_CMD -t ${REGISTRY}/${alias_name}:${TAG}"
    done
  fi

  if [ "$USE_CACHE" = false ]; then
    BUILD_CMD="$BUILD_CMD --no-cache"
  fi

  # Opt-in registry-backed buildcache (shared across dev machines).
  # Pull-only to keep local builds from clobbering shared cache; CI writes it.
  if [ "$USE_REGISTRY_CACHE" = true ]; then
    BUILD_CMD="$BUILD_CMD --cache-from type=registry,ref=${REGISTRY}/${image_name}:buildcache"
  fi

  if [ -n "$extra_args" ]; then
    BUILD_CMD="$BUILD_CMD $extra_args"
  fi

  BUILD_CMD="$BUILD_CMD -f docker/${dockerfile} ."

  echo -e "${YELLOW}Running: $BUILD_CMD${NC}"
  $BUILD_CMD

  if [ $? -eq 0 ]; then
    echo -e "${GREEN}✓ ${image_name} built successfully!${NC}"

    # For non-multi-arch builds, push separately if requested
    if [ "$MULTI_ARCH" = false ] && [ "$PUSH" = true ]; then
      echo -e "${YELLOW}Pushing ${full_image}...${NC}"
      docker push "${full_image}"
      if [ $? -ne 0 ]; then
        echo -e "${RED}✗ Push failed for ${full_image}${NC}"
        exit 1
      fi
      echo -e "${GREEN}✓ Pushed ${full_image}${NC}"

      # Also push any alias tags.
      if [ -n "$extra_tag_names" ]; then
        for alias_name in $extra_tag_names; do
          local alias_ref="${REGISTRY}/${alias_name}:${TAG}"
          echo -e "${YELLOW}Pushing ${alias_ref}...${NC}"
          docker push "${alias_ref}"
          if [ $? -ne 0 ]; then
            echo -e "${RED}✗ Push failed for ${alias_ref}${NC}"
            exit 1
          fi
          echo -e "${GREEN}✓ Pushed ${alias_ref}${NC}"
        done
      fi
    fi
    echo ""
    return 0
  else
    echo -e "${RED}✗ Build failed for ${image_name}!${NC}"
    exit 1
  fi
}

# Build images in order: stable base first, then sandbox runtime (dual-tagged
# as both sandbox and sandbox-js).
if [ "$BUILD_SANDBOX_BASE" = true ]; then
  build_image "Dockerfile.sandbox-base" "moatless-sandbox-base" \
    "--build-arg CLAUDE_CLI_VERSION=${CLAUDE_CLI_VERSION} \
     --build-arg CODEX_CLI_VERSION=${CODEX_CLI_VERSION}"
fi

if [ "$BUILD_SANDBOX" = true ]; then
  # Dockerfile.sandbox pulls entrypoint scripts + moatless-git-cred from the
  # sandbox-tools image. CI publishes it separately; build it here so the
  # COPY --from resolves for local builds without a registry pull.
  build_image "Dockerfile.sandbox-tools" "moatless-sandbox-tools" \
    "--build-context binaries=${BINARIES_DIR}"

  # Point the final image at the base and tools we just built at this TAG, not
  # the :latest defaults baked into Dockerfile.sandbox.
  build_image "Dockerfile.sandbox" "${SANDBOX_IMAGE_NAME}" \
    "--build-arg SANDBOX_BASE=${REGISTRY}/moatless-sandbox-base:${TAG} \
     --build-arg SANDBOX_TOOLS=${REGISTRY}/moatless-sandbox-tools:${TAG} \
     --build-context binaries=${BINARIES_DIR}" \
    "${SANDBOX_JS_ALIAS_NAME}"
fi

# Summary
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}Build complete!${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

if [ "$MULTI_ARCH" = true ]; then
  echo -e "\n${GREEN}Built multi-arch images (amd64 + arm64) and pushed to:${NC}"
  echo -e "  - ${REGISTRY}/moatless-sandbox-*:${TAG}"
else
  echo -e "\n${GREEN}Built images:${NC}"
  docker images | grep -E "REPOSITORY|moatless-sandbox" | head -5
  if [ "$PUSH" = true ]; then
    echo -e "\n${GREEN}Pushed to:${NC}"
    echo -e "  - ${REGISTRY}/moatless-sandbox-*:${TAG}"
  fi
fi

echo -e "\n${GREEN}To use in Kubernetes:${NC}"
echo "  K8S_SANDBOX_IMAGE=${REGISTRY}/moatless-sandbox-js:${TAG}"

if [ "$MULTI_ARCH" = true ]; then
  echo -e "\n${GREEN}To use with Docker (works on both amd64 and arm64):${NC}"
  echo "  DOCKER_SANDBOX_IMAGE=${REGISTRY}/moatless-sandbox-js:${TAG}"
fi

echo ""
echo -e "${YELLOW}Note: Use --no-cache to force rebuild all layers after code changes.${NC}"
