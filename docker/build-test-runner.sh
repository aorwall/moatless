#!/bin/bash
# Build the K8s integration test runner image
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TEMP_DIR=$(mktemp -d)

echo "=== Building K8s integration test runner image ==="
echo "Temp dir: $TEMP_DIR"

# Copy necessary files to temp directory
echo "Copying files..."
cp "$ROOT_DIR/package.json" "$TEMP_DIR/"
cp "$ROOT_DIR/bun.lock" "$TEMP_DIR/" 2>/dev/null || true
cp "$ROOT_DIR/turbo.json" "$TEMP_DIR/" 2>/dev/null || true
cp "$ROOT_DIR/tsconfig.json" "$TEMP_DIR/" 2>/dev/null || true

# Copy packages (including dist, excluding node_modules)
mkdir -p "$TEMP_DIR/packages"
for pkg in types shell-utils api-contract figma; do
  if [ -d "$ROOT_DIR/packages/$pkg" ]; then
    mkdir -p "$TEMP_DIR/packages/$pkg"
    rsync -a --exclude='node_modules' "$ROOT_DIR/packages/$pkg/" "$TEMP_DIR/packages/$pkg/"
  fi
done

# Copy backend app (including tests, excluding node_modules)
mkdir -p "$TEMP_DIR/apps"
rsync -a --exclude='node_modules' --exclude='dist' "$ROOT_DIR/apps/backend/" "$TEMP_DIR/apps/backend/"

# Create Dockerfile
cat > "$TEMP_DIR/Dockerfile" << 'EOF'
FROM oven/bun:1.1

WORKDIR /app

# Copy package files
COPY package.json bun.lock* turbo.json* tsconfig.json* ./
COPY packages ./packages
COPY apps/backend ./apps/backend

# Install dependencies (skip postinstall to avoid build issues)
RUN bun install --ignore-scripts

# Packages are pre-built and included in the image

# Force rebuild with timestamp
ARG BUILD_TIME=unknown

# Set working directory for tests
WORKDIR /app/apps/backend

# Environment variables for K8s testing
ENV K8S_ENABLED=true
ENV K8S_NAMESPACE=vibe
ENV K8S_WORKER_IMAGE=aorwall/moatless-vibe-worker:latest
ENV USE_JSON_LOGGING=true
ENV NODE_TLS_REJECT_UNAUTHORIZED=0

# Default command runs the K8s integration test
# Bun can run TypeScript directly without pre-compiling
CMD ["bun", "test", "--preload", "./src/test/setup.ts", "--timeout", "300000", \
     "src/features/workspace/drivers/__tests__/kubernetes-environment-driver.integration.test.ts"]
EOF

# Build the image for linux/amd64 (for K8s cluster) and push
echo "Building image for linux/amd64 and pushing..."
docker buildx build --platform linux/amd64 \
  --no-cache \
  -t aorwall/moatless-vibe-test-runner:latest \
  --push "$TEMP_DIR"

# Cleanup
echo "Cleaning up..."
rm -rf "$TEMP_DIR"

echo "=== Build complete ==="
echo "Image: aorwall/moatless-vibe-test-runner:latest"
echo ""
echo "To push: docker push aorwall/moatless-vibe-test-runner:latest"
