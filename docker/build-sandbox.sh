#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default values
REGISTRY="aorwall"
TAG="latest"
PLATFORM="linux/amd64,linux/arm64"
MULTI_ARCH=true
PUSH=true
USE_CACHE=true
BUILD_BASE=true
BUILD_SANDBOX_JS=true
DEV_BROWSER_DIR="${DEV_BROWSER_DIR:-}"
CLEANUP_DEV_BROWSER=false

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
    --dev-browser-dir)
      DEV_BROWSER_DIR="$2"
      shift 2
      ;;
    --no-cache)
      USE_CACHE=false
      shift
      ;;
    --only)
      case $2 in
        base)
          BUILD_SANDBOX_JS=false
          ;;
        sandbox-js)
          BUILD_BASE=false
          ;;
        *)
          echo -e "${RED}Unknown target: $2. Use: base, sandbox-js${NC}"
          exit 1
          ;;
      esac
      shift 2
      ;;
    --skip-base)
      BUILD_BASE=false
      shift
      ;;
    --help)
      echo "Usage: $0 [OPTIONS]"
      echo ""
      echo "Build moatless-vibe sandbox Docker images."
      echo ""
      echo "Images built (in order):"
      echo "  1. sandbox      - Minimal base (Ubuntu, Node.js, Claude Code)"
      echo "  2. sandbox-js   - JS/TS environment (Bun, TypeScript, Playwright)"
      echo ""
      echo "Options:"
      echo "  --tag TAG         Docker image tag (default: latest)"
      echo "  --platform ARCH   Build for specific platform (e.g., linux/amd64)"
      echo "  --multi-arch      Build for both amd64 and arm64 (requires --push)"
      echo "  --local           Build single-arch locally (no push)"
      echo "  --push            Push to Docker registry after building"
      echo "  --dev-browser-dir  Path to dev-browser plugin (default: \$DEV_BROWSER_DIR or auto-detect)"
      echo "  --no-cache        Build without using Docker build cache"
      echo "  --only TARGET     Build only one target: base or sandbox-js"
      echo "  --skip-base       Skip building base (assumes it exists)"
      echo "  --help            Show this help message"
      echo ""
      echo "Examples:"
      echo "  $0                        # Build multi-arch and push to Docker Hub (default)"
      echo "  $0 --local                # Build single-arch locally (no push)"
      echo "  $0 --only sandbox-js      # Build only sandbox-js"
      echo "  $0 --skip-base            # Build sandbox-js only (assumes base exists)"
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
  if [ "$CLEANUP_DEV_BROWSER" = true ]; then
    echo -e "${YELLOW}Cleaning up staged dev-browser plugin...${NC}"
    rm -rf dev-browser
  fi
  if [ -n "$BINARIES_DIR" ] && [ -d "$BINARIES_DIR" ]; then
    echo -e "${YELLOW}Cleaning up staged binaries...${NC}"
    rm -rf "$BINARIES_DIR"
  fi
}
trap cleanup EXIT

# Cross-compile Rust sandbox binary for target architectures (needed by Dockerfile.sandbox)
if [ "$BUILD_BASE" = true ]; then
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

# Build sandbox-js prerequisites (file-explorer, dev-browser)
if [ "$BUILD_SANDBOX_JS" = true ]; then
  # Build file-explorer (still TypeScript)
  echo -e "${YELLOW}Building file-explorer...${NC}"
  bunx turbo run build --filter=@moatless/file-explorer
  echo -e "${GREEN}✓ File-explorer build successful!${NC}"
  echo ""

  # Stage dev-browser plugin into build context
  if [ -z "$DEV_BROWSER_DIR" ]; then
    # Default: look for dev-browser relative to repo root (../../stuffs/dev-browser)
    DEV_BROWSER_DIR="$(cd ../.. && pwd)/stuffs/dev-browser"
  fi

  if [ -d "$DEV_BROWSER_DIR" ]; then
    echo -e "${YELLOW}Staging dev-browser plugin from: $DEV_BROWSER_DIR${NC}"
    rm -rf dev-browser
    cp -r "$DEV_BROWSER_DIR" dev-browser
    # Remove dev artifacts not needed in Docker image
    rm -rf dev-browser/node_modules dev-browser/.git dev-browser/extension
    CLEANUP_DEV_BROWSER=true
    echo -e "${GREEN}✓ dev-browser plugin staged${NC}"
    echo ""
  else
    echo -e "${RED}Error: dev-browser plugin not found at $DEV_BROWSER_DIR${NC}"
    echo -e "${RED}Set DEV_BROWSER_DIR env var or use --dev-browser-dir to specify location${NC}"
    exit 1
  fi
fi

# Helper function to build an image
build_image() {
  local dockerfile=$1
  local image_name=$2
  local extra_args="${3:-}"
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

  if [ "$USE_CACHE" = false ]; then
    BUILD_CMD="$BUILD_CMD --no-cache"
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
      if [ $? -eq 0 ]; then
        echo -e "${GREEN}✓ Pushed ${full_image}${NC}"
      else
        echo -e "${RED}✗ Push failed for ${full_image}${NC}"
        exit 1
      fi
    fi
    echo ""
    return 0
  else
    echo -e "${RED}✗ Build failed for ${image_name}!${NC}"
    exit 1
  fi
}

# Build images in order
if [ "$BUILD_BASE" = true ]; then
  build_image "Dockerfile.sandbox" "moatless-vibe-sandbox" "--build-context binaries=${BINARIES_DIR}"
fi

if [ "$BUILD_SANDBOX_JS" = true ]; then
  build_image "Dockerfile.sandbox-js" "moatless-vibe-sandbox-js"
fi

# Summary
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}Build complete!${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

if [ "$MULTI_ARCH" = true ]; then
  echo -e "\n${GREEN}Built multi-arch images (amd64 + arm64) and pushed to:${NC}"
  echo -e "  - ${REGISTRY}/moatless-vibe-sandbox-*:${TAG}"
else
  echo -e "\n${GREEN}Built images:${NC}"
  docker images | grep -E "REPOSITORY|moatless-vibe-sandbox" | head -5
  if [ "$PUSH" = true ]; then
    echo -e "\n${GREEN}Pushed to:${NC}"
    echo -e "  - ${REGISTRY}/moatless-vibe-sandbox-*:${TAG}"
  fi
fi

echo -e "\n${GREEN}To use in Kubernetes:${NC}"
echo "  K8S_SANDBOX_IMAGE=${REGISTRY}/moatless-vibe-sandbox-js:${TAG}"

if [ "$MULTI_ARCH" = true ]; then
  echo -e "\n${GREEN}To use with Docker (works on both amd64 and arm64):${NC}"
  echo "  DOCKER_SANDBOX_IMAGE=${REGISTRY}/moatless-vibe-sandbox-js:${TAG}"
fi

echo ""
echo -e "${YELLOW}Note: Use --no-cache to force rebuild all layers after code changes.${NC}"
