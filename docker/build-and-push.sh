#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Default values
IMAGE_NAME="ghcr.io/aorwall/moatless-vibe"
TAG="latest"
PLATFORM="linux/amd64"
PUSH=false
SKIP_BUILD=false

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
    --skip-build)
      SKIP_BUILD=true
      shift
      ;;
    --help)
      echo "Usage: $0 [OPTIONS]"
      echo "Options:"
      echo "  --tag TAG         Docker image tag (default: latest)"
      echo "  --platform ARCH   Build for specific platform (default: linux/amd64)"
      echo "  --push            Push to GitHub Container Registry after building"
      echo "  --skip-build      Skip bun build:package step"
      echo "  --help            Show this help message"
      exit 0
      ;;
    *)
      echo -e "${RED}Unknown option: $1${NC}"
      exit 1
      ;;
  esac
done

echo -e "${GREEN}Building Moatless Vibe Docker image...${NC}"
echo "Image: ${IMAGE_NAME}:${TAG}"

# Change to repository root
cd "$(dirname "$0")/.."

# Build the npm package first (unless skipped)
if [ "$SKIP_BUILD" = false ]; then
  echo -e "${YELLOW}Building npm package...${NC}"
  bun run build:package
  if [ $? -ne 0 ]; then
    echo -e "${RED}✗ Package build failed!${NC}"
    exit 1
  fi
  echo -e "${GREEN}✓ Package build successful!${NC}"
else
  echo -e "${YELLOW}Skipping package build...${NC}"
fi

# Create temporary combined Dockerfile
TEMP_DOCKERFILE="docker/Dockerfile.app.combined"
echo -e "${YELLOW}Creating combined Dockerfile...${NC}"
cat > "$TEMP_DOCKERFILE" <<'EOF'
# Combined Dockerfile (base + app) for local builds
FROM oven/bun:1.2.18-alpine

# Install minimal system dependencies
RUN apk add --no-cache \
    git \
    ca-certificates \
    tini

# Create data directory with proper permissions for bun user
RUN mkdir -p /data && chown bun:bun /data

WORKDIR /app

# Copy entrypoint script with proper permissions
COPY docker/docker-entrypoint-app.sh /usr/local/bin/
RUN chmod 755 /usr/local/bin/docker-entrypoint-app.sh

# Copy the packaged moatless-vibe application
COPY dist /app/dist
COPY bin /app/bin
COPY apps/backend/drizzle /app/drizzle
COPY apps/backend/drizzle-postgres /app/drizzle-postgres
COPY package.json README.md LICENSE /app/

# Remove prepare/postinstall scripts that cause issues in Docker
RUN node -e "const pkg = require('./package.json'); delete pkg.scripts.prepare; delete pkg.scripts.postinstall; require('fs').writeFileSync('package.json', JSON.stringify(pkg, null, 2))"

# Switch to non-root user for runtime
USER bun

# Environment variables
ENV NODE_ENV=production
ENV WORKER_MODE=false
ENV PORT=8000
ENV STATIC_CLIENT_PATH=/app/dist/client

# Expose port
EXPOSE 8000

# Use tini as init system for proper signal handling
ENTRYPOINT ["/sbin/tini", "--"]
CMD ["/usr/local/bin/docker-entrypoint-app.sh"]
EOF

# Build Docker image
echo -e "${YELLOW}Building Docker image...${NC}"
docker build --platform "$PLATFORM" -f "$TEMP_DOCKERFILE" -t "${IMAGE_NAME}:${TAG}" .

if [ $? -eq 0 ]; then
  echo -e "${GREEN}✓ Docker build successful!${NC}"

  # Show image info
  echo -e "\n${GREEN}Image details:${NC}"
  docker images | grep -E "REPOSITORY|${IMAGE_NAME}" | head -2

  # Push if requested
  if [ "$PUSH" = true ]; then
    echo -e "\n${YELLOW}Pushing image to GitHub Container Registry...${NC}"
    docker push "${IMAGE_NAME}:${TAG}"
    if [ $? -eq 0 ]; then
      echo -e "${GREEN}✓ Push successful!${NC}"
      echo -e "${GREEN}Image: ${IMAGE_NAME}:${TAG}${NC}"
    else
      echo -e "${RED}✗ Push failed!${NC}"
      echo -e "${YELLOW}Make sure you have write:packages scope:${NC}"
      echo "  gh auth refresh -h github.com -s write:packages"
      echo "  gh auth token | docker login ghcr.io -u USERNAME --password-stdin"
      rm -f "$TEMP_DOCKERFILE"
      exit 1
    fi
  else
    echo -e "\n${YELLOW}To push the image, run:${NC}"
    echo "  docker push ${IMAGE_NAME}:${TAG}"
    echo -e "\n${YELLOW}Or re-run with --push flag:${NC}"
    echo "  $0 --push"
  fi
else
  echo -e "${RED}✗ Docker build failed!${NC}"
  rm -f "$TEMP_DOCKERFILE"
  exit 1
fi

# Cleanup temporary Dockerfile
rm -f "$TEMP_DOCKERFILE"
echo -e "${GREEN}✓ Done!${NC}"
