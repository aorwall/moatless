#!/bin/bash
set -e

# Initialize firewall rules if running with privileges
# Disabled by default - uncomment to enable firewall rules
# if [ -f /usr/local/bin/init-firewall.sh ]; then
#     sudo /usr/local/bin/init-firewall.sh || true
# fi

# Ensure data directory exists with correct permissions
if [ ! -d "$DATA_DIR" ]; then
    mkdir -p "$DATA_DIR"
fi

# Ensure workspace directory exists with correct permissions
if [ ! -d "$WORKSPACE_DIR" ]; then
    mkdir -p "$WORKSPACE_DIR"
fi

# Start the backend in worker mode
echo "Starting Moatless backend in worker mode..."
echo "Worker API URL: ${WORKER_API_URL:-http://localhost:8000}"
echo "Worker Task ID: ${WORKER_TASK_ID:-not set}"
echo "Workspace Dir: $WORKSPACE_DIR"
echo "Data Dir: $DATA_DIR"
echo "Port: $PORT"

# Change to app directory and start the backend
cd /app/apps/backend
exec node dist/index.js