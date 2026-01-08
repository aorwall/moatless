#!/bin/sh
set -e

# Ensure data directory exists
if [ ! -d "/data" ]; then
    mkdir -p /data 2>/dev/null || true
fi

exec bun /app/dist/server.js
