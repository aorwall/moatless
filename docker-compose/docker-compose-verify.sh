#!/bin/bash
set -e

echo "🔍 Moatless Vibe Health Check"
echo "=============================="
echo ""

# Get the directory where this script is located and change to it
# This ensures all relative paths (env files, data dirs) resolve correctly
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Detect docker-compose command (v1 vs v2)
if command -v docker-compose >/dev/null 2>&1; then
    DOCKER_COMPOSE="docker-compose"
else
    DOCKER_COMPOSE="docker compose"
fi

# Always use the compose file from the script's directory
COMPOSE_FILE="$SCRIPT_DIR/docker-compose.yml"

# Load environment variables
if [ -f .env.docker-compose ]; then
    set -a
    source .env.docker-compose
    set +a
fi

BACKEND_PORT=${BACKEND_PORT:-8000}
FRONTEND_PORT=${FRONTEND_PORT:-3000}

# Check if services are running
echo "📋 Checking service status..."
$DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file .env.docker-compose ps
echo ""

# Initialize counters
HEALTHY_COUNT=0
TOTAL_COUNT=0

# Check PostgreSQL
echo "🐘 PostgreSQL health..."
TOTAL_COUNT=$((TOTAL_COUNT + 1))
if $DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file .env.docker-compose exec -T postgres pg_isready -U moatless > /dev/null 2>&1; then
    echo "   ✅ PostgreSQL is healthy"
    HEALTHY_COUNT=$((HEALTHY_COUNT + 1))
else
    echo "   ❌ PostgreSQL is not responding"
    echo "   Troubleshooting: $DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file .env.docker-compose logs postgres"
fi

# Check Redis
echo ""
echo "🔴 Redis health..."
TOTAL_COUNT=$((TOTAL_COUNT + 1))
if $DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file .env.docker-compose exec -T redis redis-cli ping 2>/dev/null | grep -q PONG; then
    echo "   ✅ Redis is healthy"
    HEALTHY_COUNT=$((HEALTHY_COUNT + 1))
else
    echo "   ❌ Redis is not responding"
    echo "   Troubleshooting: $DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file .env.docker-compose logs redis"
fi

# Check Backend
echo ""
echo "🖥️  Backend health..."
TOTAL_COUNT=$((TOTAL_COUNT + 1))
BACKEND_RESPONSE=$(curl -s http://localhost:${BACKEND_PORT}/health 2>/dev/null || echo "")
if [ -n "$BACKEND_RESPONSE" ]; then
    echo "   ✅ Backend is healthy"
    echo "   Response: $BACKEND_RESPONSE"
    HEALTHY_COUNT=$((HEALTHY_COUNT + 1))
else
    echo "   ❌ Backend is not responding"
    echo "   Troubleshooting: $DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file .env.docker-compose logs backend"
fi

# Check Frontend
echo ""
echo "🎨 Frontend health..."
TOTAL_COUNT=$((TOTAL_COUNT + 1))
if curl -s http://localhost:${FRONTEND_PORT} > /dev/null 2>&1; then
    echo "   ✅ Frontend is healthy"
    HEALTHY_COUNT=$((HEALTHY_COUNT + 1))
else
    echo "   ❌ Frontend is not responding"
    echo "   Troubleshooting: $DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file .env.docker-compose logs frontend"
fi

# Check Docker socket access
echo ""
echo "🐳 Docker socket access..."
TOTAL_COUNT=$((TOTAL_COUNT + 1))
if $DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file .env.docker-compose exec -T backend docker ps > /dev/null 2>&1; then
    echo "   ✅ Backend can access Docker socket"
    HEALTHY_COUNT=$((HEALTHY_COUNT + 1))
else
    echo "   ❌ Backend cannot access Docker socket"
    echo "   This is required for creating sandbox containers"
    echo "   Troubleshooting: Check that /var/run/docker.sock is mounted correctly"
fi

# Check database connection
echo ""
echo "🗄️  Database connection..."
TOTAL_COUNT=$((TOTAL_COUNT + 1))
if $DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file .env.docker-compose exec -T postgres psql -U moatless -d moatless -c "SELECT 1" > /dev/null 2>&1; then
    echo "   ✅ Database connection successful"
    HEALTHY_COUNT=$((HEALTHY_COUNT + 1))

    # Show table count
    TABLE_COUNT=$($DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file .env.docker-compose exec -T postgres psql -U moatless -d moatless -t -c "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public'" 2>/dev/null | tr -d ' ')
    echo "   Tables: $TABLE_COUNT (expected: 13 after migrations)"
else
    echo "   ❌ Database connection failed"
fi

# Check Redis streams
echo ""
echo "📊 Redis event streams..."
STREAM_INFO=$($DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file .env.docker-compose exec -T redis redis-cli INFO keyspace 2>/dev/null || echo "")
if [ -n "$STREAM_INFO" ]; then
    echo "   ✅ Redis is configured for event streaming"
else
    echo "   ℹ️  No active event streams (will be created when tasks run)"
fi

# Summary
echo ""
echo "=============================="
echo "Health Check Summary: $HEALTHY_COUNT/$TOTAL_COUNT services healthy"

if [ $HEALTHY_COUNT -eq $TOTAL_COUNT ]; then
    echo "✅ All systems operational!"
    exit 0
else
    echo "⚠️  Some services are unhealthy. See above for troubleshooting steps."
    exit 1
fi
