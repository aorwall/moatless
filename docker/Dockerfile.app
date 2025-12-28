# moatless-vibe: Lightweight web application image
# For the full development environment (worker), use Dockerfile.worker

FROM oven/bun:1.2.18-alpine AS runtime

# Install minimal system dependencies
RUN apk add --no-cache \
    git \
    ca-certificates \
    tini

# The bun:alpine image already has a 'bun' user with UID 1000
# We'll use that user for consistency with worker image (ubuntu UID 1000)

WORKDIR /app

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

# Create data directory with proper permissions for bun user
RUN mkdir -p /data && chown bun:bun /data

# Switch to non-root user for runtime
USER bun

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
