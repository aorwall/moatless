#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Default values
IMAGE_NAME="moatless-vibe"
REGISTRY="ghcr.io/aorwall"
TAG="latest"
PLATFORM="linux/amd64,linux/arm64"
MULTI_ARCH=true
PUSH=true
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
    --local)
      MULTI_ARCH=false
      PUSH=false
      PLATFORM="linux/amd64"
      shift
      ;;
    --no-cache)
      USE_CACHE=false
      shift
      ;;
    --help)
      echo "Usage: $0 [OPTIONS]"
      echo ""
      echo "Build moatless-vibe app Docker image (nginx + static frontend)."
      echo ""
      echo "Options:"
      echo "  --tag TAG         Docker image tag (default: latest)"
      echo "  --platform ARCH   Build for specific platform (default: linux/amd64,linux/arm64)"
      echo "  --local           Build single-arch locally (no push)"
      echo "  --push            Push to Docker registry after building"
      echo "  --no-cache        Build without using Docker build cache"
      echo "  --help            Show this help message"
      echo ""
      echo "Examples:"
      echo "  $0                        # Build multi-arch and push to GHCR + Docker Hub (default)"
      echo "  $0 --local                # Build single-arch locally (no push)"
      echo "  $0 --no-cache             # Force rebuild all layers (use after code changes)"
      exit 0
      ;;
    *)
      echo -e "${RED}Unknown option: $1${NC}"
      exit 1
      ;;
  esac
done

# Image names for both registries
GHCR_NAME="${REGISTRY}/${IMAGE_NAME}:${TAG}"
DOCKERHUB_NAME="aorwall/${IMAGE_NAME}:${TAG}"

echo -e "${GREEN}Building Moatless Vibe App Docker image...${NC}"
echo "GHCR: ${GHCR_NAME}"
echo "Docker Hub: ${DOCKERHUB_NAME}"

# Change to repository root
cd "$(dirname "$0")/.."

# Rebuild the project before Docker build to catch errors early
echo -e "${YELLOW}Rebuilding project...${NC}"
bun install --frozen-lockfile
bunx turbo run build --filter=@moatless/frontend
echo -e "${GREEN}✓ Project build successful${NC}"
echo ""

# Validate multi-arch requires push
if [ "$MULTI_ARCH" = true ] && [ "$PUSH" = false ]; then
  echo -e "${RED}Error: Multi-arch builds require --push (multi-arch images can't be loaded locally)${NC}"
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

# Build command - use buildx for multi-arch, regular docker build otherwise
if [ "$MULTI_ARCH" = true ]; then
  BUILD_CMD="docker buildx build --platform $PLATFORM --push -t ${GHCR_NAME} -t ${DOCKERHUB_NAME}"
else
  BUILD_CMD="docker build -t ${GHCR_NAME} -t ${DOCKERHUB_NAME}"
  if [ -n "$PLATFORM" ]; then
    BUILD_CMD="$BUILD_CMD --platform $PLATFORM"
  fi
fi

# Add cache options
if [ "$USE_CACHE" = false ]; then
  BUILD_CMD="$BUILD_CMD --no-cache"
fi

# Add build arguments
BUILD_CMD="$BUILD_CMD -f docker/Dockerfile.app ."

# Execute build
echo -e "${YELLOW}Running: $BUILD_CMD${NC}"
$BUILD_CMD

if [ $? -eq 0 ]; then
  echo -e "${GREEN}✓ Build successful!${NC}"

  # For non-multi-arch builds, push separately if requested
  if [ "$MULTI_ARCH" = false ] && [ "$PUSH" = true ]; then
    echo -e "\n${YELLOW}Pushing ${GHCR_NAME}...${NC}"
    docker push "${GHCR_NAME}"
    if [ $? -eq 0 ]; then
      echo -e "${GREEN}✓ Pushed ${GHCR_NAME}${NC}"
    else
      echo -e "${RED}✗ Push failed for ${GHCR_NAME}${NC}"
      exit 1
    fi
    echo -e "${YELLOW}Pushing ${DOCKERHUB_NAME}...${NC}"
    docker push "${DOCKERHUB_NAME}"
    if [ $? -eq 0 ]; then
      echo -e "${GREEN}✓ Pushed ${DOCKERHUB_NAME}${NC}"
    else
      echo -e "${RED}✗ Push failed for ${DOCKERHUB_NAME}${NC}"
      exit 1
    fi
  fi

  # Summary
  echo -e "\n${GREEN}Build complete!${NC}"
  if [ "$MULTI_ARCH" = true ]; then
    echo -e "Built multi-arch images (amd64 + arm64) and pushed to:"
    echo -e "  - GHCR: ${GHCR_NAME}"
    echo -e "  - Docker Hub: ${DOCKERHUB_NAME}"
  else
    echo -e "\n${GREEN}Built images:${NC}"
    docker images | grep -E "REPOSITORY|${IMAGE_NAME}" | head -3
    if [ "$PUSH" = true ]; then
      echo -e "\n${GREEN}Pushed to:${NC}"
      echo -e "  - GHCR: ${GHCR_NAME}"
      echo -e "  - Docker Hub: ${DOCKERHUB_NAME}"
    fi
  fi
else
  echo -e "${RED}✗ Build failed!${NC}"
  exit 1
fi
