FROM node:20-slim

WORKDIR /app

# Install git for LocalWorkspace functionality and curl for SSE streaming
RUN apt-get update && apt-get install -y \
    git \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Install pnpm
RUN corepack enable && corepack prepare pnpm@latest --activate

# Copy workspace configuration
COPY pnpm-workspace.yaml ./
COPY .npmrc ./
COPY pnpm-lock.yaml ./

# Copy package files first for better caching
COPY package.json ./
COPY apps/backend/package.json ./apps/backend/
COPY packages/shared/package.json ./packages/shared/

# Install dependencies
RUN pnpm install --frozen-lockfile

# Install the Claude Code SDK binary by running pnpm postinstall manually
# This ensures the binary is downloaded and installed
RUN cd /app && pnpm postinstall || true

# Copy source code
COPY tsconfig.json ./
COPY packages/shared ./packages/shared
COPY apps/backend ./apps/backend

# Build shared package and backend
RUN pnpm --filter @moatless/shared build
RUN pnpm --filter backend build

# Create workspace directory for worker mode
RUN mkdir -p /workspace && chmod 777 /workspace

EXPOSE 8000

WORKDIR /app/apps/backend

# Start in normal or worker mode based on environment
CMD ["node", "dist/index.js"]