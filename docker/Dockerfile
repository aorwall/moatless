# Multi-stage build for efficiency
# Stage 1: Build the application
FROM node:20 AS builder

# Install pnpm
RUN npm install -g pnpm@10.7.1

# Copy source code
WORKDIR /app
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml turbo.json tsconfig.json tsconfig.node.json ./
COPY packages ./packages
COPY apps ./apps

# Install dependencies and build
RUN pnpm install --frozen-lockfile
RUN pnpm build

# Stage 2: Runtime image
FROM node:20

ARG TZ
ENV TZ="$TZ"

# Install basic development tools and iptables/ipset
RUN apt update && apt install -y less \
  git \
  procps \
  sudo \
  fzf \
  zsh \
  man-db \
  unzip \
  gnupg2 \
  gh \
  iptables \
  ipset \
  iproute2 \
  dnsutils \
  aggregate \
  jq

# Ensure default node user has access to /usr/local/share
RUN mkdir -p /usr/local/share/npm-global && \
  chown -R node:node /usr/local/share

ARG USERNAME=node

# Persist bash history.
RUN SNIPPET="export PROMPT_COMMAND='history -a' && export HISTFILE=/commandhistory/.bash_history" \
  && mkdir /commandhistory \
  && touch /commandhistory/.bash_history \
  && chown -R $USERNAME /commandhistory

# Set `DEVCONTAINER` environment variable to help with orientation
ENV DEVCONTAINER=true

# Create workspace and config directories and set permissions
RUN mkdir -p /workspace /home/node/.claude /data /app && \
  chown -R node:node /workspace /home/node/.claude /data /app

WORKDIR /workspace

RUN ARCH=$(dpkg --print-architecture) && \
  wget "https://github.com/dandavison/delta/releases/download/0.18.2/git-delta_0.18.2_${ARCH}.deb" && \
  sudo dpkg -i "git-delta_0.18.2_${ARCH}.deb" && \
  rm "git-delta_0.18.2_${ARCH}.deb"

# Copy built application from builder stage
# Copy root node_modules (includes pnpm store)
COPY --from=builder --chown=node:node /app/node_modules /app/node_modules

# Copy shared package
COPY --from=builder --chown=node:node /app/packages/shared/dist /app/packages/shared/dist
COPY --from=builder --chown=node:node /app/packages/shared/package.json /app/packages/shared/package.json
COPY --from=builder --chown=node:node /app/packages/shared/node_modules /app/packages/shared/node_modules

# Copy backend app
COPY --from=builder --chown=node:node /app/apps/backend/dist /app/apps/backend/dist
COPY --from=builder --chown=node:node /app/apps/backend/package.json /app/apps/backend/package.json
COPY --from=builder --chown=node:node /app/apps/backend/node_modules /app/apps/backend/node_modules

# Set up non-root user
USER node

# Install global packages
ENV NPM_CONFIG_PREFIX=/usr/local/share/npm-global
ENV PATH=$PATH:/usr/local/share/npm-global/bin

# Set the default shell to zsh rather than sh
ENV SHELL=/bin/zsh

# Default powerline10k theme
RUN sh -c "$(wget -O- https://github.com/deluan/zsh-in-docker/releases/download/v1.2.0/zsh-in-docker.sh)" -- \
  -p git \
  -p fzf \
  -a "source /usr/share/doc/fzf/examples/key-bindings.zsh" \
  -a "source /usr/share/doc/fzf/examples/completion.zsh" \
  -a "export PROMPT_COMMAND='history -a' && export HISTFILE=/commandhistory/.bash_history" \
  -x

# Install Claude
RUN npm install -g @anthropic-ai/claude-code

# Copy and set up firewall script
COPY docker/init-firewall.sh /usr/local/bin/
USER root
RUN chmod +x /usr/local/bin/init-firewall.sh && \
  echo "node ALL=(root) NOPASSWD: /usr/local/bin/init-firewall.sh" > /etc/sudoers.d/node-firewall && \
  chmod 0440 /etc/sudoers.d/node-firewall

# Create entrypoint script
COPY docker/docker-entrypoint.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

# Switch back to node user
USER node

# Set environment variables for worker mode
ENV NODE_ENV=production
ENV WORKER_MODE=true
ENV DATA_DIR=/data
ENV WORKSPACE_DIR=/workspace
ENV PORT=8001

# Expose worker port
EXPOSE 8001

# Set entrypoint
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]