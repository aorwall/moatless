#!/bin/bash
set -e

echo "Building local base image..."
docker build -t moatless-vibe-base:local -f docker/Dockerfile.base .

echo "Building application..."
bun run build:package

echo "Starting docker compose..."
# Use desktop-linux builder which can access local images
docker buildx use desktop-linux 2>/dev/null || true
docker compose -f docker-compose/docker-compose.yml \
  -f docker-compose/docker-compose.dev.yml \
  --env-file docker-compose/.env.docker-compose \
  up -d --build

echo "Done! Backend running at http://localhost:8000"
