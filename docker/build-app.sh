#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Default values
IMAGE_NAME="aorwall/moatless-vibe"
TAG="latest"
PLATFORM="linux/amd64,linux/arm64"
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
      echo "Options:"
      echo "  --tag TAG         Docker image tag (default: latest)"
      echo "  --platform ARCH   Build for specific platform (default: linux/amd64,linux/arm64)"
      echo "                    For single platform: --platform linux/amd64"
      echo "  --push            Push to Docker registry after building"
      echo "  --no-cache        Build without using Docker build cache"
      echo "  --help            Show this help message"
      echo ""
      echo "Note: Multi-platform builds use docker buildx and will only load the native"
      echo "      architecture locally unless --push is specified."
      exit 0
      ;;
    *)
      echo -e "${RED}Unknown option: $1${NC}"
      exit 1
      ;;
  esac
done

echo -e "${GREEN}Building Moatless Vibe App Docker image...${NC}"
echo "Image: ${IMAGE_NAME}:${TAG}"

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

# Build command - use buildx for multi-platform builds
if [[ "$PLATFORM" == *","* ]]; then
  # Multi-platform build requires buildx
  BUILD_CMD="docker buildx build --platform $PLATFORM"

  # Multi-platform builds must be pushed or loaded
  if [ "$PUSH" = false ]; then
    BUILD_CMD="$BUILD_CMD --load"
    echo -e "${YELLOW}Note: Multi-platform build will only load the native architecture image.${NC}"
    echo -e "${YELLOW}To build and push all platforms, use --push flag.${NC}"
  fi
else
  # Single platform build uses regular docker build
  BUILD_CMD="docker build"

  # Add platform if specified
  if [ -n "$PLATFORM" ]; then
    BUILD_CMD="$BUILD_CMD --platform $PLATFORM"
  fi
fi

# Add cache options
if [ "$USE_CACHE" = false ]; then
  BUILD_CMD="$BUILD_CMD --no-cache"
fi

# Add build arguments
BUILD_CMD="$BUILD_CMD -f docker/Dockerfile.app -t ${IMAGE_NAME}:${TAG} ."

# Execute build
echo -e "${YELLOW}Running: $BUILD_CMD${NC}"
$BUILD_CMD

if [ $? -eq 0 ]; then
  echo -e "${GREEN}✓ Build successful!${NC}"

  # Show image info
  echo -e "\n${GREEN}Image details:${NC}"
  docker images | grep -E "REPOSITORY|${IMAGE_NAME}" | head -2

  # Push if requested (only for non-buildx builds)
  if [ "$PUSH" = true ] && [[ "$PLATFORM" != *","* ]]; then
    echo -e "\n${YELLOW}Pushing image to registry...${NC}"
    docker push "${IMAGE_NAME}:${TAG}"
    if [ $? -eq 0 ]; then
      echo -e "${GREEN}✓ Push successful!${NC}"
    else
      echo -e "${RED}✗ Push failed!${NC}"
      exit 1
    fi
  elif [ "$PUSH" = true ]; then
    echo -e "${GREEN}✓ Multi-platform images pushed during build!${NC}"
  fi
else
  echo -e "${RED}✗ Build failed!${NC}"
  exit 1
fi
