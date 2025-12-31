#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Default values
IMAGE_NAME="aorwall/moatless-vibe-worker-js"
TAG="latest"
PLATFORM="linux/amd64"
PUSH=false
USE_CACHE=true

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
    --help)
      echo "Usage: $0 [OPTIONS]"
      echo ""
      echo "Build the lightweight JavaScript/TypeScript worker image."
      echo "This image is ~10x smaller than the full worker and is optimized for JS/TS projects."
      echo ""
      echo "Options:"
      echo "  --tag TAG         Docker image tag (default: latest)"
      echo "  --platform ARCH   Build for specific platform (e.g., linux/amd64,linux/arm64)"
      echo "  --push            Push to Docker registry after building"
      echo "  --no-cache        Build without using Docker build cache"
      echo "  --help            Show this help message"
      echo ""
      echo "Examples:"
      echo "  $0                           # Build for local platform"
      echo "  $0 --platform linux/amd64    # Build for amd64"
      echo "  $0 --push                    # Build and push to registry"
      echo "  $0 --platform linux/amd64 --push  # Build for amd64 and push"
      exit 0
      ;;
    *)
      echo -e "${RED}Unknown option: $1${NC}"
      exit 1
      ;;
  esac
done

echo -e "${GREEN}Building Moatless JS/TS Worker Docker image (slim)...${NC}"
echo "Image: ${IMAGE_NAME}:${TAG}"
echo "This is the lightweight image for JavaScript/TypeScript projects only."

# Change to repository root
cd "$(dirname "$0")/.."

# Build the npm package first
echo -e "${YELLOW}Building npm package...${NC}"
bun build:package
if [ $? -ne 0 ]; then
  echo -e "${RED}✗ Package build failed!${NC}"
  exit 1
fi
echo -e "${GREEN}✓ Package build successful!${NC}"

# Determine build command based on platform and push settings
if [ -n "$PLATFORM" ] && [ "$PUSH" = true ]; then
  # Use buildx for cross-platform builds with push
  BUILD_CMD="docker buildx build --platform $PLATFORM"
  if [ "$USE_CACHE" = false ]; then
    BUILD_CMD="$BUILD_CMD --no-cache"
  fi
  BUILD_CMD="$BUILD_CMD -f docker/Dockerfile.worker-js -t ${IMAGE_NAME}:${TAG} --push ."
else
  # Standard docker build
  BUILD_CMD="docker build"
  if [ -n "$PLATFORM" ]; then
    BUILD_CMD="$BUILD_CMD --platform $PLATFORM"
  fi
  if [ "$USE_CACHE" = false ]; then
    BUILD_CMD="$BUILD_CMD --no-cache"
  fi
  BUILD_CMD="$BUILD_CMD -f docker/Dockerfile.worker-js -t ${IMAGE_NAME}:${TAG} ."
fi

# Execute build
echo -e "${YELLOW}Running: $BUILD_CMD${NC}"
$BUILD_CMD

if [ $? -eq 0 ]; then
  echo -e "${GREEN}✓ Build successful!${NC}"

  # Show image info (only if not using buildx with push)
  if [ -z "$PLATFORM" ] || [ "$PUSH" = false ]; then
    echo -e "\n${GREEN}Image details:${NC}"
    docker images | grep -E "REPOSITORY|${IMAGE_NAME}" | head -2
  fi

  # Push if requested and not already pushed via buildx
  if [ "$PUSH" = true ] && [ -z "$PLATFORM" ]; then
    echo -e "\n${YELLOW}Pushing image to registry...${NC}"
    docker push "${IMAGE_NAME}:${TAG}"
    if [ $? -eq 0 ]; then
      echo -e "${GREEN}✓ Push successful!${NC}"
    else
      echo -e "${RED}✗ Push failed!${NC}"
      exit 1
    fi
  fi

  echo -e "\n${GREEN}To use this slim image for JS/TS repos:${NC}"
  echo "K8S_WORKER_IMAGE=${IMAGE_NAME}:${TAG}"
  echo ""
  echo -e "${YELLOW}Note: This image only supports JavaScript/TypeScript projects.${NC}"
  echo "For Python, Go, Rust, or Java projects, use the full worker image instead."
else
  echo -e "${RED}✗ Build failed!${NC}"
  exit 1
fi
