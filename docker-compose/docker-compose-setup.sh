#!/bin/bash
set -e

echo "🚀 Moatless Docker Compose Setup"
echo "======================================"
echo ""

# Check prerequisites
echo "🔍 Checking prerequisites..."
command -v docker >/dev/null 2>&1 || { echo "❌ Docker is required but not installed. Visit https://docs.docker.com/get-docker/"; exit 1; }
command -v docker-compose >/dev/null 2>&1 || command -v docker compose >/dev/null 2>&1 || { echo "❌ Docker Compose is required but not installed."; exit 1; }

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

echo "✅ Docker and Docker Compose are installed"
echo ""

# Create .env if it doesn't exist
if [ ! -f ../.env ]; then
    echo "📝 Creating .env from template..."
    cp ../.env.example ../.env
    # `|` as the delimiter: a hex secret never contains it.
    sed -i.bak "s|^TASK_TOKEN_SECRET=your_task_token_secret_here$|TASK_TOKEN_SECRET=$(openssl rand -hex 32)|" ../.env
    rm -f ../.env.bak
    echo "🔑 Generated TASK_TOKEN_SECRET"

    echo ""
    echo "⚠️  IMPORTANT: You must edit .env and set the following required values:"
    echo ""
    echo "   1. CLAUDE_CODE_OAUTH_TOKEN - Get from https://claude.ai/settings/code"
    echo "   2. POSTGRES_PASSWORD - Change from default password"
    echo ""
    read -p "Press Enter to open .env in your editor..."
    ${EDITOR:-nano} ../.env
    echo ""
fi

# Validate required environment variables
echo "🔍 Validating configuration..."

# Source the env file
if [ -f ../.env ]; then
    set -a
    source ../.env
    set +a
fi

VALIDATION_FAILED=false

if [ -z "$CLAUDE_CODE_OAUTH_TOKEN" ] || [ "$CLAUDE_CODE_OAUTH_TOKEN" = "your_claude_oauth_token_here" ]; then
    echo "❌ CLAUDE_CODE_OAUTH_TOKEN is required. Get token by running claude setup-token"
    VALIDATION_FAILED=true
fi

if [ -z "$TASK_TOKEN_SECRET" ] || [ "$TASK_TOKEN_SECRET" = "your_task_token_secret_here" ]; then
    echo "❌ TASK_TOKEN_SECRET is required. Generate with: openssl rand -hex 32"
    VALIDATION_FAILED=true
fi

if [ "$VALIDATION_FAILED" = true ]; then
    echo ""
    echo "Please edit .env and set the required values, then run this script again."
    exit 1
fi

echo "✅ Configuration validated"
echo ""

# Create data directory with per-task subdirectory
echo "📁 Creating data directories..."
mkdir -p ./data/tasks
echo "✅ Data directories created:"
echo "   - ./data (base directory)"
echo "   - ./data/tasks (per-task storage)"
echo ""

# Pull images
echo "📥 Pulling Docker images..."
$DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file ../.env pull

echo ""
echo "🐳 Starting services with Docker Compose..."
$DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file ../.env up -d

echo ""
echo "⏳ Waiting for services to be healthy..."
sleep 5

# Show service status
$DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file ../.env ps

echo ""
echo "✅ Setup complete!"
echo ""
echo "🌐 Services are running:"
echo "   - Frontend: http://localhost:${FRONTEND_PORT:-3000}"
echo "   - Backend:  http://localhost:${BACKEND_PORT:-8080}"
echo ""
echo "📊 Useful commands:"
echo "   View logs:        $DOCKER_COMPOSE -f \"$COMPOSE_FILE\" --env-file ../.env logs -f"
echo "   View backend:     $DOCKER_COMPOSE -f \"$COMPOSE_FILE\" --env-file ../.env logs -f backend"
echo "   Stop services:    $DOCKER_COMPOSE -f \"$COMPOSE_FILE\" --env-file ../.env down"
echo "   Restart services: $DOCKER_COMPOSE -f \"$COMPOSE_FILE\" --env-file ../.env restart"
echo "   Health check:     $SCRIPT_DIR/docker-compose-verify.sh"
echo ""
echo "📖 For more information, see docs/docker-compose-guide.md"
