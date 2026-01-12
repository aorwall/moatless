#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default values
REGISTRY="ghcr.io/aorwall"
TAG="latest"
PLATFORM="linux/amd64"
MULTI_ARCH=true
PUSH=true
USE_CACHE=true
BUILD_BASE=true
BUILD_SANDBOX=true
BUILD_SANDBOX_JS=true

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
    --no-cache)
      USE_CACHE=false
      shift
      ;;
    --only)
      case $2 in
        base)
          BUILD_SANDBOX=false
          BUILD_SANDBOX_JS=false
          ;;
        sandbox)
          BUILD_BASE=false
          BUILD_SANDBOX_JS=false
          ;;
        sandbox-js)
          BUILD_BASE=false
          BUILD_SANDBOX=false
          ;;
        *)
          echo -e "${RED}Unknown target: $2. Use: base, sandbox, sandbox-js${NC}"
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
      echo "  1. sandbox-base    - Minimal base (Ubuntu, Node.js, Claude Code)"
      echo "  2. sandbox         - Full dev environment (Python, Rust, Go, Java)"
      echo "  3. sandbox-js      - JS/TS environment (Bun, TypeScript, Playwright)"
      echo ""
      echo "Options:"
      echo "  --tag TAG         Docker image tag (default: latest)"
      echo "  --platform ARCH   Build for specific platform (e.g., linux/amd64)"
      echo "  --multi-arch      Build for both amd64 and arm64 (requires --push)"
      echo "  --push            Push to Docker registry after building"
      echo "  --no-cache        Build without using Docker build cache"
      echo "  --only TARGET     Build only one target: base, sandbox, or sandbox-js"
      echo "  --skip-base       Skip building base (assumes it exists)"
      echo "  --help            Show this help message"
      echo ""
      echo "Examples:"
      echo "  $0                        # Build all images for amd64"
      echo "  $0 --multi-arch --push    # Build for amd64+arm64 and push"
      echo "  $0 --only sandbox-js      # Build only sandbox-js (assumes base exists)"
      echo "  $0 --skip-base            # Build sandbox + sandbox-js (assumes base exists)"
      echo "  $0 --push                 # Build all and push to registry"
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

# Build sandbox app first (needed for sandbox images)
if [ "$BUILD_SANDBOX" = true ] || [ "$BUILD_SANDBOX_JS" = true ]; then
  echo -e "${YELLOW}Building sandbox application...${NC}"
  bun run build
  if [ $? -ne 0 ]; then
    echo -e "${RED}✗ Build failed!${NC}"
    exit 1
  fi
  echo -e "${GREEN}✓ Build successful!${NC}"
  echo ""
fi

# Helper function to build an image
build_image() {
  local dockerfile=$1
  local image_name=$2
  local full_name="${REGISTRY}/${image_name}:${TAG}"

  echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
  echo -e "${GREEN}Building ${image_name}...${NC}"
  echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

  # Use buildx for multi-arch, regular docker build otherwise
  if [ "$MULTI_ARCH" = true ]; then
    BUILD_CMD="docker buildx build --platform $PLATFORM --push"
  else
    BUILD_CMD="docker build"
    if [ -n "$PLATFORM" ]; then
      BUILD_CMD="$BUILD_CMD --platform $PLATFORM"
    fi
  fi

  if [ "$USE_CACHE" = false ]; then
    BUILD_CMD="$BUILD_CMD --no-cache"
  fi

  BUILD_CMD="$BUILD_CMD -f docker/${dockerfile} -t ${full_name} ."

  echo -e "${YELLOW}Running: $BUILD_CMD${NC}"
  $BUILD_CMD

  if [ $? -eq 0 ]; then
    echo -e "${GREEN}✓ ${image_name} built successfully!${NC}"

    # For non-multi-arch builds, push separately if requested
    if [ "$MULTI_ARCH" = false ] && [ "$PUSH" = true ]; then
      echo -e "${YELLOW}Pushing ${full_name}...${NC}"
      docker push "${full_name}"
      if [ $? -eq 0 ]; then
        echo -e "${GREEN}✓ Pushed ${full_name}${NC}"
      else
        echo -e "${RED}✗ Push failed for ${full_name}${NC}"
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
  build_image "Dockerfile.sandbox-base" "moatless-vibe-sandbox-base"
fi

if [ "$BUILD_SANDBOX" = true ]; then
  build_image "Dockerfile.sandbox" "moatless-vibe-sandbox"
fi

if [ "$BUILD_SANDBOX_JS" = true ]; then
  build_image "Dockerfile.sandbox-js" "moatless-vibe-sandbox-js"
fi

# Summary
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}Build complete!${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

if [ "$MULTI_ARCH" = true ]; then
  echo -e "\n${GREEN}Built multi-arch images (amd64 + arm64) and pushed to registry${NC}"
else
  echo -e "\n${GREEN}Built images:${NC}"
  docker images | grep -E "REPOSITORY|moatless-vibe-sandbox" | head -5
fi

echo -e "\n${GREEN}To use in Kubernetes:${NC}"
echo "  K8S_SANDBOX_IMAGE=${REGISTRY}/moatless-vibe-sandbox:${TAG}"

if [ "$MULTI_ARCH" = true ]; then
  echo -e "\n${GREEN}To use with Docker (works on both amd64 and arm64):${NC}"
  echo "  DOCKER_SANDBOX_IMAGE=${REGISTRY}/moatless-vibe-sandbox-js:${TAG}"
fi

echo ""
echo -e "${YELLOW}Note: Docker layer caching means unchanged layers won't be rebuilt.${NC}"
