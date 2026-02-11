#!/bin/bash
set -e

# Build and push server container images to Docker Hub.
#
# Images:
#   aorwall/moatless-vibe-files-server    (file explorer)
#   aorwall/moatless-vibe-preview-server  (preview/custom servers)

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

TAG="latest"
PLATFORM="linux/amd64,linux/arm64"
MULTI_ARCH=true
PUSH=true
USE_CACHE=true
BUILD_FILES=true
BUILD_PREVIEW=true

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
    --local)
      MULTI_ARCH=false
      PUSH=false
      case "$(uname -m)" in
        x86_64|amd64)  PLATFORM="linux/amd64" ;;
        aarch64|arm64) PLATFORM="linux/arm64" ;;
        *) echo -e "${RED}Unsupported architecture: $(uname -m)${NC}"; exit 1 ;;
      esac
      shift
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
        files)
          BUILD_PREVIEW=false
          ;;
        preview)
          BUILD_FILES=false
          ;;
        *)
          echo -e "${RED}Unknown target: $2. Use: files, preview${NC}"
          exit 1
          ;;
      esac
      shift 2
      ;;
    --help)
      echo "Usage: $0 [OPTIONS]"
      echo ""
      echo "Build server container images for dynamic sandbox provisioning."
      echo ""
      echo "Images built:"
      echo "  1. moatless-vibe-files-server    - File explorer server"
      echo "  2. moatless-vibe-preview-server  - Preview/custom server runtime"
      echo ""
      echo "Options:"
      echo "  --tag TAG         Docker image tag (default: latest)"
      echo "  --platform ARCH   Build for specific platform"
      echo "  --local           Build single-arch locally (no push)"
      echo "  --push            Push to Docker Hub after building"
      echo "  --no-cache        Build without Docker cache"
      echo "  --only TARGET     Build only one: files or preview"
      echo "  --help            Show this help"
      echo ""
      echo "Examples:"
      echo "  $0                        # Build multi-arch and push to Docker Hub"
      echo "  $0 --local                # Build single-arch locally"
      echo "  $0 --only files           # Build only files-server"
      echo "  $0 --tag v0.2.0           # Build with specific tag"
      exit 0
      ;;
    *)
      echo -e "${RED}Unknown option: $1${NC}"
      exit 1
      ;;
  esac
done

cd "$(dirname "$0")/.."

if [ "$MULTI_ARCH" = true ] && [ "$PUSH" = false ]; then
  echo -e "${RED}Error: --multi-arch requires --push${NC}"
  exit 1
fi

if [ "$MULTI_ARCH" = true ]; then
  BUILDER_NAME="moatless-multiarch"
  if ! docker buildx inspect "$BUILDER_NAME" > /dev/null 2>&1; then
    echo -e "${YELLOW}Creating buildx builder...${NC}"
    docker buildx create --name "$BUILDER_NAME" --driver docker-container --bootstrap
  fi
  docker buildx use "$BUILDER_NAME"
fi

# Build file-explorer dist (needed by files-server image)
if [ "$BUILD_FILES" = true ]; then
  echo -e "${YELLOW}Building file-explorer...${NC}"
  bunx turbo run build --filter=@moatless/file-explorer
  echo -e "${GREEN}✓ File-explorer build successful${NC}"
  echo ""
fi

build_image() {
  local dockerfile=$1
  local image_name=$2
  local dockerhub_name="aorwall/${image_name}:${TAG}"

  echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
  echo -e "${GREEN}Building ${image_name}...${NC}"
  echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

  if [ "$MULTI_ARCH" = true ]; then
    BUILD_CMD="docker buildx build --platform $PLATFORM --push -t ${dockerhub_name}"
  else
    BUILD_CMD="docker build -t ${dockerhub_name}"
    if [ -n "$PLATFORM" ]; then
      BUILD_CMD="$BUILD_CMD --platform $PLATFORM"
    fi
  fi

  if [ "$USE_CACHE" = false ]; then
    BUILD_CMD="$BUILD_CMD --no-cache"
  fi

  BUILD_CMD="$BUILD_CMD -f docker/${dockerfile} ."

  echo -e "${YELLOW}Running: $BUILD_CMD${NC}"
  $BUILD_CMD

  if [ $? -eq 0 ]; then
    echo -e "${GREEN}✓ ${image_name} built successfully!${NC}"
    if [ "$MULTI_ARCH" = false ] && [ "$PUSH" = true ]; then
      echo -e "${YELLOW}Pushing ${dockerhub_name}...${NC}"
      docker push "${dockerhub_name}"
      echo -e "${GREEN}✓ Pushed ${dockerhub_name}${NC}"
    fi
    echo ""
  else
    echo -e "${RED}✗ Build failed for ${image_name}!${NC}"
    exit 1
  fi
}

if [ "$BUILD_FILES" = true ]; then
  build_image "Dockerfile.files-server" "moatless-vibe-files-server"
fi

if [ "$BUILD_PREVIEW" = true ]; then
  build_image "Dockerfile.preview-server" "moatless-vibe-preview-server"
fi

echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}Server images build complete!${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""
echo -e "${GREEN}Images on Docker Hub:${NC}"
[ "$BUILD_FILES" = true ] && echo "  aorwall/moatless-vibe-files-server:${TAG}"
[ "$BUILD_PREVIEW" = true ] && echo "  aorwall/moatless-vibe-preview-server:${TAG}"
echo ""
