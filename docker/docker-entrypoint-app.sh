#!/bin/sh
set -e

# Ensure data directory exists
if [ ! -d "/data" ]; then
    mkdir -p /data 2>/dev/null || true
fi

# Start the moatless-vibe web application
echo "Starting Moatless Vibe..."
echo "Port: ${PORT:-8000}"
echo "Database: ${DATABASE_PROVIDER:-sqlite}"

exec bun /app/dist/server.js
