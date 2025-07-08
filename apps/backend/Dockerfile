FROM node:20-slim

WORKDIR /app

# Install git for LocalWorkspace functionality
RUN apt-get update && apt-get install -y \
    git \
    && rm -rf /var/lib/apt/lists/*

# Install pnpm
RUN corepack enable && corepack prepare pnpm@latest --activate

# Copy workspace configuration
COPY pnpm-workspace.yaml ./
COPY .npmrc ./

# Copy package files first for better caching
COPY package.json ./
COPY apps/backend/package.json ./apps/backend/
COPY packages/shared/package.json ./packages/shared/

# Install dependencies
RUN pnpm install --frozen-lockfile

# Copy source code
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