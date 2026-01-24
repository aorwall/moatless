# moatless-vibe: Lightweight web application image
# Self-contained - no separate base image needed
#
# Published as: ghcr.io/aorwall/moatless-vibe:latest

FROM oven/bun:1.3.6-slim

# Install minimal system dependencies including Docker CLI
RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    ca-certificates \
    tini \
    && rm -rf /var/lib/apt/lists/*

# Install Docker CLI
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    gnupg \
    && curl -fsSL https://download.docker.com/linux/debian/gpg | gpg --dearmor -o /usr/share/keyrings/docker-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/docker-archive-keyring.gpg] https://download.docker.com/linux/debian bookworm stable" > /etc/apt/sources.list.d/docker.list \
    && apt-get update && apt-get install -y --no-install-recommends docker-ce-cli \
    && rm -rf /var/lib/apt/lists/*

# Create data directory with proper permissions for bun user
RUN mkdir -p /data && chown bun:bun /data

WORKDIR /app

# Copy entrypoint script with proper permissions (readable by all)
COPY docker/docker-entrypoint-app.sh /usr/local/bin/
RUN chmod 755 /usr/local/bin/docker-entrypoint-app.sh

# Copy and install the packaged moatless-vibe application
COPY dist /app/dist
COPY bin /app/bin
COPY apps/backend/drizzle-postgres /app/drizzle-postgres
COPY package.json README.md LICENSE /app/

# Remove prepare/postinstall scripts that cause issues in Docker
RUN node -e "const pkg = require('./package.json'); delete pkg.scripts.prepare; delete pkg.scripts.postinstall; require('fs').writeFileSync('package.json', JSON.stringify(pkg, null, 2))"

# Note: Running as root for Docker socket access on macOS Docker Desktop
# For production, use proper user permissions or Docker socket proxy

# Environment variables
ENV NODE_ENV=production
ENV WORKER_MODE=false
ENV PORT=8000
ENV STATIC_CLIENT_PATH=/app/dist/client

# Expose port
EXPOSE 8000

# Use tini as init system for proper signal handling
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["/usr/local/bin/docker-entrypoint-app.sh"]
