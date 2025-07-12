#!/bin/bash

# Moatless Docker Container Cleanup Script
# This script cleans up any Docker containers created by moatless

echo "🧹 Cleaning up moatless Docker containers..."

# Find all containers created by moatless
echo "🔍 Looking for moatless containers..."
MOATLESS_CONTAINERS=$(docker ps -a --filter "label=created-by=moatless" --format "{{.Names}}")

if [ -z "$MOATLESS_CONTAINERS" ]; then
  echo "✅ No moatless containers found"
else
  echo "🚮 Found moatless containers:"
  echo "$MOATLESS_CONTAINERS"
  
  # Show container details before cleanup
  echo ""
  echo "📋 Container details:"
  docker ps -a --filter "label=created-by=moatless" --format "table {{.Names}}\t{{.Image}}\t{{.Status}}\t{{.Labels}}"
  echo ""
  
  # Stop containers (ignore errors if already stopped)
  echo "🛑 Stopping containers..."
  echo "$MOATLESS_CONTAINERS" | xargs -r docker stop 2>/dev/null || true
  
  # Remove containers
  echo "🗑️  Removing containers..."
  echo "$MOATLESS_CONTAINERS" | xargs -r docker rm 2>/dev/null || true
  
  echo "✅ Moatless containers cleaned up"
fi

# Also check for legacy test containers (from before labels were added)
echo ""
echo "🔍 Looking for legacy test containers..."
LEGACY_CONTAINERS=$(docker ps -a --filter "name=basic-test-" --filter "name=debug-container-" --filter "name=moatless-task-" --format "{{.Names}}")

if [ ! -z "$LEGACY_CONTAINERS" ]; then
  echo "🚮 Found legacy test containers:"
  echo "$LEGACY_CONTAINERS"
  echo "$LEGACY_CONTAINERS" | xargs -r docker stop 2>/dev/null || true
  echo "$LEGACY_CONTAINERS" | xargs -r docker rm 2>/dev/null || true
  echo "✅ Legacy containers cleaned up"
else
  echo "✅ No legacy containers found"
fi

echo ""
echo "🎉 Docker cleanup completed!"

# Show remaining containers for verification
echo ""
echo "📋 All remaining containers:"
docker ps --format "table {{.Names}}\t{{.Image}}\t{{.Status}}"