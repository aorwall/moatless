# moatless-vibe: Lightweight web application image
# For the full development environment (worker), use Dockerfile.worker
#
# This image inherits from the pre-built base image for fast builds.
# Base image contains: bun, git, ca-certificates, tini, docker-cli
# To rebuild base: run "Publish Base Images" workflow with "app"

FROM ghcr.io/aorwall/moatless-vibe-base:latest

# Copy entrypoint script with proper permissions (readable by all)
COPY docker/docker-entrypoint-app.sh /usr/local/bin/
RUN chmod 755 /usr/local/bin/docker-entrypoint-app.sh

# Copy and install the packaged moatless-vibe application
COPY dist /app/dist
COPY bin /app/bin
COPY apps/backend/drizzle /app/drizzle
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
ENTRYPOINT ["/sbin/tini", "--"]
CMD ["/usr/local/bin/docker-entrypoint-app.sh"]
