#!/bin/bash
set -e

echo "🚀 Moatless Vibe Docker Compose Setup"
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

# Create .env.docker-compose if it doesn't exist
if [ ! -f .env.docker-compose ]; then
    echo "📝 Creating .env.docker-compose from template..."
    cp .env.docker-compose.example .env.docker-compose

    echo ""
    echo "⚠️  IMPORTANT: You must edit .env.docker-compose and set the following required values:"
    echo ""
    echo "   1. CLAUDE_CODE_OAUTH_TOKEN - Get from https://claude.ai/settings/code"
    echo "   2. TASK_TOKEN_SECRET - Generate with: openssl rand -hex 32"
    echo "   3. POSTGRES_PASSWORD - Change from default password"
    echo ""
    read -p "Press Enter to open .env.docker-compose in your editor..."
    ${EDITOR:-nano} .env.docker-compose
    echo ""
fi

# Validate required environment variables
echo "🔍 Validating configuration..."

# Source the env file
if [ -f .env.docker-compose ]; then
    set -a
    source .env.docker-compose
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
    echo "Please edit .env.docker-compose and set the required values, then run this script again."
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
$DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file .env.docker-compose pull

echo ""
echo "🐳 Starting services with Docker Compose..."
$DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file .env.docker-compose up -d

echo ""
echo "⏳ Waiting for services to be healthy..."
sleep 5

# Show service status
$DOCKER_COMPOSE -f "$COMPOSE_FILE" --env-file .env.docker-compose ps

echo ""
echo "✅ Setup complete!"
echo ""
echo "🌐 Services are running:"
echo "   - Frontend: http://localhost:${FRONTEND_PORT:-3000}"
echo "   - Backend:  http://localhost:${BACKEND_PORT:-8080}"
echo ""
echo "📊 Useful commands:"
echo "   View logs:        $DOCKER_COMPOSE -f \"$COMPOSE_FILE\" --env-file .env.docker-compose logs -f"
echo "   View backend:     $DOCKER_COMPOSE -f \"$COMPOSE_FILE\" --env-file .env.docker-compose logs -f backend"
echo "   Stop services:    $DOCKER_COMPOSE -f \"$COMPOSE_FILE\" --env-file .env.docker-compose down"
echo "   Restart services: $DOCKER_COMPOSE -f \"$COMPOSE_FILE\" --env-file .env.docker-compose restart"
echo "   Health check:     $SCRIPT_DIR/docker-compose-verify.sh"
echo ""
echo "📖 For more information, see docs/docker-compose-guide.md"
