#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Default values
BASE_IMAGE_NAME="moatless-vibe-base"
TAG="latest"
PLATFORM=""
PUSH=false

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
    --help)
      echo "Usage: $0 [OPTIONS]"
      echo "Options:"
      echo "  --tag TAG         Docker image tag (default: latest)"
      echo "  --platform ARCH   Build for specific platform (e.g., linux/amd64,linux/arm64)"
      echo "  --push            Push to Docker registry after building"
      echo "  --help            Show this help message"
      exit 0
      ;;
    *)
      echo -e "${RED}Unknown option: $1${NC}"
      exit 1
      ;;
  esac
done

echo -e "${GREEN}Building Moatless Base System Docker image...${NC}"
echo "Base Image: ${BASE_IMAGE_NAME}:${TAG}"

# Change to repository root
cd "$(dirname "$0")/.."

# Build command for base system only
BUILD_CMD="docker build"

# Add platform if specified
if [ -n "$PLATFORM" ]; then
  BUILD_CMD="$BUILD_CMD --platform $PLATFORM"
fi

# Build only the base-system stage
BUILD_CMD="$BUILD_CMD --target base-system -f docker/Dockerfile -t ${BASE_IMAGE_NAME}:${TAG} ."

# Execute build
echo -e "${YELLOW}Running: $BUILD_CMD${NC}"
$BUILD_CMD

if [ $? -eq 0 ]; then
  echo -e "${GREEN}✓ Base system build successful!${NC}"
  
  # Show image info
  echo -e "\n${GREEN}Base image details:${NC}"
  docker images | grep -E "REPOSITORY|${BASE_IMAGE_NAME}" | head -2
  
  # Push if requested
  if [ "$PUSH" = true ]; then
    echo -e "\n${YELLOW}Pushing base image to registry...${NC}"
    docker push "${BASE_IMAGE_NAME}:${TAG}"
    if [ $? -eq 0 ]; then
      echo -e "${GREEN}✓ Push successful!${NC}"
    else
      echo -e "${RED}✗ Push failed!${NC}"
      exit 1
    fi
  fi
  
  echo -e "\n${GREEN}Base image ready. You can now build the full image faster using:${NC}"
  echo "./docker/build-worker.sh --tag latest"
else
  echo -e "${RED}✗ Base system build failed!${NC}"
  exit 1
fi