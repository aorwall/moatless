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
PUSH=false
USE_CACHE=true
BUILD_BASE=true
BUILD_WORKER=true
BUILD_WORKER_JS=true

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
    --no-cache)
      USE_CACHE=false
      shift
      ;;
    --only)
      case $2 in
        base)
          BUILD_WORKER=false
          BUILD_WORKER_JS=false
          ;;
        worker)
          BUILD_BASE=false
          BUILD_WORKER_JS=false
          ;;
        worker-js)
          BUILD_BASE=false
          BUILD_WORKER=false
          ;;
        *)
          echo -e "${RED}Unknown target: $2. Use: base, worker, worker-js${NC}"
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
      echo "Build moatless-vibe worker Docker images."
      echo ""
      echo "Images built (in order):"
      echo "  1. worker-base    - Minimal base (Ubuntu, Node.js, Claude Code)"
      echo "  2. worker         - Full dev environment (Python, Rust, Go, Java)"
      echo "  3. worker-js      - JS/TS environment (Bun, TypeScript, Playwright)"
      echo ""
      echo "Options:"
      echo "  --tag TAG         Docker image tag (default: latest)"
      echo "  --platform ARCH   Build for specific platform (e.g., linux/amd64)"
      echo "  --push            Push to Docker registry after building"
      echo "  --no-cache        Build without using Docker build cache"
      echo "  --only TARGET     Build only one target: base, worker, or worker-js"
      echo "  --skip-base       Skip building base (assumes it exists)"
      echo "  --help            Show this help message"
      echo ""
      echo "Examples:"
      echo "  $0                        # Build all images"
      echo "  $0 --only worker-js       # Build only worker-js (assumes base exists)"
      echo "  $0 --skip-base            # Build worker + worker-js (assumes base exists)"
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

# Build npm package first (needed for worker images)
if [ "$BUILD_WORKER" = true ] || [ "$BUILD_WORKER_JS" = true ]; then
  echo -e "${YELLOW}Building npm package...${NC}"
  bun build:package
  if [ $? -ne 0 ]; then
    echo -e "${RED}✗ Package build failed!${NC}"
    exit 1
  fi
  echo -e "${GREEN}✓ Package build successful!${NC}"
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

  BUILD_CMD="docker build"

  if [ -n "$PLATFORM" ]; then
    BUILD_CMD="$BUILD_CMD --platform $PLATFORM"
  fi

  if [ "$USE_CACHE" = false ]; then
    BUILD_CMD="$BUILD_CMD --no-cache"
  fi

  BUILD_CMD="$BUILD_CMD -f docker/${dockerfile} -t ${full_name} ."

  echo -e "${YELLOW}Running: $BUILD_CMD${NC}"
  $BUILD_CMD

  if [ $? -eq 0 ]; then
    echo -e "${GREEN}✓ ${image_name} built successfully!${NC}"

    if [ "$PUSH" = true ]; then
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
  build_image "Dockerfile.worker-base" "moatless-vibe-worker-base"
fi

if [ "$BUILD_WORKER" = true ]; then
  build_image "Dockerfile.worker" "moatless-vibe-worker"
fi

if [ "$BUILD_WORKER_JS" = true ]; then
  build_image "Dockerfile.worker-js" "moatless-vibe-worker-js"
fi

# Summary
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}Build complete!${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

echo -e "\n${GREEN}Built images:${NC}"
docker images | grep -E "REPOSITORY|moatless-vibe-worker" | head -5

echo -e "\n${GREEN}To use in Kubernetes:${NC}"
echo "  K8S_WORKER_IMAGE=${REGISTRY}/moatless-vibe-worker:${TAG}"
echo ""
echo -e "${YELLOW}Note: Docker layer caching means unchanged layers won't be rebuilt.${NC}"
