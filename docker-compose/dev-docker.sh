#!/bin/bash
set -e

# Change to the project root directory
cd "$(dirname "$0")/.."

echo "Building local base image..."
docker build -t moatless-vibe-base:local -f docker/Dockerfile.base .

echo "Building application..."
bun run build:package

echo "Building local sandbox-base image..."
docker build -t moatless-vibe-sandbox-base:local -f docker/Dockerfile.sandbox-base .

echo "Building local sandbox-js image..."
docker build -t moatless-vibe-sandbox-js:local \
  --build-arg BASE_IMAGE=moatless-vibe-sandbox-base:local \
  -f docker/Dockerfile.sandbox-js .

# Export sandbox image for docker-compose to use
export SANDBOX_IMAGE=moatless-vibe-sandbox-js:local

echo "Starting docker compose..."
# Use desktop-linux builder which can access local images
docker buildx use desktop-linux 2>/dev/null || true
docker compose -f docker-compose/docker-compose.yml \
  -f docker-compose/docker-compose.dev.yml \
  --env-file docker-compose/.env.docker-compose \
  up -d --build

echo "Done! Backend running at http://localhost:8000"
