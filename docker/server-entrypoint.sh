#!/bin/bash
set -euo pipefail

# Wait for the sandbox container to finish core initialization (git clone, credentials).
# The sandbox writes this sentinel file after InitPhase::Ready.
READY_FILE="${DATA_DIR:-/opt/moatless/data}/.workspace-ready"

echo "Waiting for workspace readiness ($READY_FILE)..."
while [ ! -f "$READY_FILE" ]; do
    sleep 2
done
echo "Workspace ready, starting server..."

exec "$@"
