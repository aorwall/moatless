#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default values
CONTAINER_NAME="sandbox-dev"
NETWORK_NAME="moatless"
LOCAL_IMAGE="aorwall/moatless-sandbox-js:latest"
GIT_CLONE_URL="https://github.com/aorwall/moatable"
HOST_PORT_API=""
HOST_PORT_PREVIEW=""
SKIP_BUILD=false

# Function to find an available port
find_available_port() {
  local port
  # Try to find an available port using Python (cross-platform)
  if command -v python3 &> /dev/null; then
    port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("", 0)); print(s.getsockname()[1]); s.close()')
  elif command -v python &> /dev/null; then
    port=$(python -c 'import socket; s=socket.socket(); s.bind(("", 0)); print(s.getsockname()[1]); s.close()')
  else
    # Fallback: pick a random port in ephemeral range and hope it's free
    port=$((49152 + RANDOM % 16383))
  fi
  echo "$port"
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
  case $1 in
    --name)
      CONTAINER_NAME="$2"
      shift 2
      ;;
    --repo)
      GIT_CLONE_URL="$2"
      shift 2
      ;;
    --api-port)
      HOST_PORT_API="$2"
      shift 2
      ;;
    --preview-port)
      HOST_PORT_PREVIEW="$2"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=true
      shift
      ;;
    --help)
      echo "Usage: $0 [OPTIONS]"
      echo ""
      echo "Build and run a sandbox-js container interactively with bash."
      echo ""
      echo "This script will:"
      echo "  1. Build the sandbox application (bun run build)"
      echo "  2. Build the Docker image locally"
      echo "  3. Run the container with an interactive bash shell"
      echo ""
      echo "Options:"
      echo "  --name NAME         Container name (default: sandbox-dev)"
      echo "  --repo URL          Git clone URL (default: https://github.com/aorwall/moatable)"
      echo "  --api-port PORT     Host port for sandbox API (default: random available)"
      echo "  --preview-port PORT Host port for preview server (default: random available)"
      echo "  --skip-build        Skip building, use existing local image"
      echo "  --help              Show this help message"
      echo ""
      echo "Examples:"
      echo "  $0                              # Build and run with defaults"
      echo "  $0 --skip-build                 # Run without rebuilding"
      echo "  $0 --repo https://github.com/user/repo  # Clone a different repo"
      echo "  $0 --api-port 8001              # Use specific API port"
      echo ""
      echo "Inside the container:"
      echo "  - Check env vars: env | grep -E 'CLAUDE|PORT|GIT'"
      echo "  - Check cloned repo: ls /opt/moatless/workspace"
      echo "  - Start sandbox: /usr/local/bin/sandbox"
      exit 0
      ;;
    *)
      echo -e "${RED}Unknown option: $1${NC}"
      exit 1
      ;;
  esac
done

# Change to repository root to find .env file
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR/.."

echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}Moatless Sandbox Runner${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

# Build step
if [ "$SKIP_BUILD" = false ]; then
  echo -e ""
  echo -e "${YELLOW}Building sandbox images via build-sandbox.sh --local...${NC}"
  if ! "$SCRIPT_DIR/build-sandbox.sh" --local; then
    echo -e "${RED}✗ Build failed!${NC}"
    exit 1
  fi
  echo -e "${GREEN}✓ Build successful${NC}"
  echo -e ""
else
  echo -e ""
  echo -e "${YELLOW}Skipping build (using existing image: $LOCAL_IMAGE)${NC}"
  if ! docker image inspect "$LOCAL_IMAGE" > /dev/null 2>&1; then
    echo -e "${RED}Error: Local image '$LOCAL_IMAGE' not found. Run without --skip-build first.${NC}"
    exit 1
  fi
  echo -e ""
fi

# Load environment variables from .env file
if [ -f ".env" ]; then
  echo -e "${YELLOW}Loading .env file...${NC}"
  # Export variables from .env, filtering comments and empty lines
  # Using grep and sed to handle values with spaces properly
  while IFS='=' read -r key value; do
    # Skip empty lines and comments
    [[ -z "$key" || "$key" =~ ^# ]] && continue
    # Remove leading/trailing whitespace from key
    key=$(echo "$key" | xargs)
    # Remove surrounding quotes from value if present
    value=$(echo "$value" | sed -e 's/^"//' -e 's/"$//' -e "s/^'//" -e "s/'$//")
    # Export the variable
    export "$key=$value"
  done < .env
else
  echo -e "${YELLOW}Warning: .env file not found${NC}"
fi

# Check for Claude authentication
if [ -z "$CLAUDE_CODE_OAUTH_TOKEN" ] && [ -z "$ANTHROPIC_API_KEY" ]; then
  echo -e "${RED}Error: No Claude authentication configured.${NC}"
  echo -e "${RED}Set CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY in .env file.${NC}"
  exit 1
fi

# Assign random available ports if not specified
if [ -z "$HOST_PORT_API" ]; then
  HOST_PORT_API=$(find_available_port)
fi
if [ -z "$HOST_PORT_PREVIEW" ]; then
  HOST_PORT_PREVIEW=$(find_available_port)
fi

# Create Docker network if it doesn't exist
if ! docker network inspect "$NETWORK_NAME" > /dev/null 2>&1; then
  echo -e "${YELLOW}Creating Docker network: $NETWORK_NAME${NC}"
  docker network create "$NETWORK_NAME"
fi

# Stop and remove existing container if it exists
if docker ps -a --filter "name=^/${CONTAINER_NAME}$" --format "{{.ID}}" | grep -q .; then
  echo -e "${YELLOW}Removing existing container: $CONTAINER_NAME${NC}"
  docker rm -f "$CONTAINER_NAME" > /dev/null 2>&1 || true
fi

echo -e ""
echo -e "${GREEN}Configuration:${NC}"
echo -e "  Container: ${BLUE}$CONTAINER_NAME${NC}"
echo -e "  Image: ${BLUE}$LOCAL_IMAGE${NC}"
echo -e "  Network: ${BLUE}$NETWORK_NAME${NC}"
echo -e "  Git URL: ${BLUE}$GIT_CLONE_URL${NC}"
echo -e "  API Port: ${BLUE}$HOST_PORT_API${NC} (sandbox API)"
echo -e "  Preview Port: ${BLUE}$HOST_PORT_PREVIEW${NC} (preview server)"
echo -e ""

# Build docker run command
DOCKER_CMD="docker run -it --rm"
DOCKER_CMD+=" --name $CONTAINER_NAME"
DOCKER_CMD+=" --network $NETWORK_NAME"

# Port mappings
DOCKER_CMD+=" -p $HOST_PORT_API:8001"
DOCKER_CMD+=" -p $HOST_PORT_PREVIEW:3000"

# Sandbox environment variables
DOCKER_CMD+=" -e PORT=8001"
DOCKER_CMD+=" -e LOG_LEVEL=debug"
DOCKER_CMD+=" -e HOME=/home/moatless"
DOCKER_CMD+=" -e WORKSPACE_DIR=/opt/moatless/workspace"
DOCKER_CMD+=" -e DATA_DIR=/opt/moatless/data"

# Git clone URL
DOCKER_CMD+=" -e GIT_CLONE_URL=$GIT_CLONE_URL"

# Claude authentication
if [ -n "$CLAUDE_CODE_OAUTH_TOKEN" ]; then
  DOCKER_CMD+=" -e CLAUDE_CODE_OAUTH_TOKEN=$CLAUDE_CODE_OAUTH_TOKEN"
fi

if [ -n "$ANTHROPIC_API_KEY" ]; then
  DOCKER_CMD+=" -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY"
fi

# Image and entrypoint override for bash
DOCKER_CMD+=" --entrypoint /bin/bash"
DOCKER_CMD+=" $LOCAL_IMAGE"

echo -e "${GREEN}Starting sandbox container...${NC}"
echo -e "${YELLOW}Note: Repository will be cloned on container start${NC}"
echo -e ""
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}Inside the container, you can:${NC}"
echo -e "  1. Check environment: ${YELLOW}env | grep -E 'CLAUDE|PORT|GIT'${NC}"
echo -e "  2. Clone the repo manually: ${YELLOW}git clone \$GIT_CLONE_URL \$WORKSPACE_DIR/<repo-name>${NC}"
echo -e "  3. Start sandbox server: ${YELLOW}/usr/local/bin/sandbox${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e ""

# Run the container
$DOCKER_CMD
