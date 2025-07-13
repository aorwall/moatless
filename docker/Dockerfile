# moatless-universal: Comprehensive development environment
# Based on OpenAI codex-universal (MIT License)
# https://github.com/openai/codex-universal

### Stage 1: Build the application ###
FROM ubuntu:24.04 AS builder

ENV LANG="C.UTF-8"
ENV DEBIAN_FRONTEND=noninteractive

# Install base development tools and dependencies
RUN apt-get update \
    && apt-get upgrade -y \
    && apt-get install -y --no-install-recommends \
        binutils \
        build-essential \
        curl \
        git \
        git-lfs \
        gnupg2 \
        jq \
        ca-certificates \
        software-properties-common \
        wget \
    && rm -rf /var/lib/apt/lists/*

# Install Node.js 20 via NodeSource repository
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y nodejs

# Install pnpm with corepack
RUN corepack enable && corepack prepare pnpm@10.7.1 --activate

# Copy source code
WORKDIR /app
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml turbo.json tsconfig.json tsconfig.node.json .npmrc ./
COPY packages ./packages
COPY apps ./apps

# Install dependencies and build
RUN pnpm install --frozen-lockfile

# Install the Claude Code SDK binary by running pnpm postinstall manually
# This ensures the binary is downloaded and installed
RUN pnpm postinstall || true

RUN pnpm build

### Stage 2: Universal development runtime ###
FROM ubuntu:24.04

ENV LANG="C.UTF-8"
ENV HOME=/workspace
ENV DEBIAN_FRONTEND=noninteractive

# Install comprehensive system packages (based on codex-universal)
RUN apt-get update \
    && apt-get upgrade -y \
    && apt-get install -y --no-install-recommends \
        # Base system tools
        binutils \
        sudo \
        build-essential \
        bzr \
        curl \
        default-libmysqlclient-dev \
        dnsutils \
        gettext \
        git \
        git-lfs \
        gnupg2 \
        inotify-tools \
        iputils-ping \
        jq \
        # Development libraries
        libbz2-dev \
        libc6 \
        libc6-dev \
        libcurl4-openssl-dev \
        libdb-dev \
        libedit2 \
        libffi-dev \
        libgcc-13-dev \
        libgcc1 \
        libgdbm-compat-dev \
        libgdbm-dev \
        libgdiplus \
        libgssapi-krb5-2 \
        liblzma-dev \
        libncurses-dev \
        libncursesw5-dev \
        libnss3-dev \
        libpq-dev \
        libpsl-dev \
        libpython3-dev \
        libreadline-dev \
        libsqlite3-dev \
        libssl-dev \
        libstdc++-13-dev \
        libunwind8 \
        libuuid1 \
        libxml2-dev \
        libz3-dev \
        # Additional tools
        make \
        moreutils \
        netcat-openbsd \
        openssh-client \
        pkg-config \
        protobuf-compiler \
        python3-pip \
        ripgrep \
        rsync \
        software-properties-common \
        sqlite3 \
        swig3.0 \
        tk-dev \
        tzdata \
        unzip \
        vim \
        wget \
        zip \
        # Additional dev tools for moatless
        less \
        procps \
        fzf \
        zsh \
        man-db \
        gh \
        iptables \
        ipset \
        iproute2 \
        aggregate \
    && rm -rf /var/lib/apt/lists/*

# Install Node.js 20 via NodeSource repository
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install Python via pyenv
ENV PYENV_ROOT="/opt/pyenv"
ENV PATH="$PYENV_ROOT/bin:$PATH"
RUN git clone https://github.com/pyenv/pyenv.git $PYENV_ROOT \
    && cd $PYENV_ROOT && git checkout v2.4.21 \
    && echo 'export PYENV_ROOT="/opt/pyenv"' >> /etc/profile \
    && echo 'export PATH="$PYENV_ROOT/bin:$PATH"' >> /etc/profile \
    && echo 'eval "$(pyenv init -)"' >> /etc/profile

# Install Python versions
RUN eval "$(pyenv init -)" \
    && pyenv install 3.12.8 \
    && pyenv global 3.12.8

# Install Rust
ENV RUSTUP_HOME="/opt/rust"
ENV CARGO_HOME="/opt/cargo"
ENV PATH="/opt/cargo/bin:$PATH"
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path \
    && echo 'export PATH="/opt/cargo/bin:$PATH"' >> /etc/profile

# Install Go
ENV GO_VERSION=1.23.8
ENV GOROOT="/usr/local/go"
ENV GOPATH="/opt/go"
ENV PATH="$GOROOT/bin:$GOPATH/bin:$PATH"
RUN wget -q "https://go.dev/dl/go${GO_VERSION}.linux-amd64.tar.gz" \
    && tar -C /usr/local -xzf "go${GO_VERSION}.linux-amd64.tar.gz" \
    && rm "go${GO_VERSION}.linux-amd64.tar.gz" \
    && mkdir -p $GOPATH \
    && echo 'export PATH="/usr/local/go/bin:/opt/go/bin:$PATH"' >> /etc/profile

# Install Java OpenJDK 21 and Maven
RUN apt-get update \
    && apt-get install -y openjdk-21-jdk maven \
    && rm -rf /var/lib/apt/lists/* \
    && ln -s /usr/lib/jvm/java-21-openjdk-* /usr/lib/jvm/default-java
ENV JAVA_HOME="/usr/lib/jvm/default-java"

# Configure existing ubuntu user (UID 1000)
ARG USERNAME=ubuntu
ARG USER_UID=1000
ARG USER_GID=$USER_UID

RUN echo "$USERNAME ALL=(root) NOPASSWD:ALL" > /etc/sudoers.d/$USERNAME \
    && chmod 0440 /etc/sudoers.d/$USERNAME \
    && usermod -s /bin/zsh $USERNAME

# Set up pnpm and npm
RUN corepack enable && corepack prepare pnpm@10.7.1 --activate

# Create directories and set permissions
RUN mkdir -p /workspace /home/$USERNAME/.claude /data /app /commandhistory \
    && touch /commandhistory/.bash_history \
    && chown -R $USERNAME:$USERNAME /workspace /home/$USERNAME/.claude /data /app /commandhistory \
    && chmod -R 755 /workspace /data /app

# Install git-delta
RUN ARCH=$(dpkg --print-architecture) && \
    wget "https://github.com/dandavison/delta/releases/download/0.18.2/git-delta_0.18.2_${ARCH}.deb" && \
    dpkg -i "git-delta_0.18.2_${ARCH}.deb" && \
    rm "git-delta_0.18.2_${ARCH}.deb"

# Install Claude Code CLI as root
RUN npm install -g @anthropic-ai/claude-code

# Switch to non-root user
USER $USERNAME

WORKDIR /workspace

# Set up shell environment
ENV SHELL=/bin/zsh
RUN sh -c "$(wget -O- https://github.com/deluan/zsh-in-docker/releases/download/v1.2.0/zsh-in-docker.sh)" -- \
    -p git \
    -p fzf \
    -a "source /usr/share/doc/fzf/examples/key-bindings.zsh" \
    -a "source /usr/share/doc/fzf/examples/completion.zsh" \
    -a "export PROMPT_COMMAND='history -a' && export HISTFILE=/commandhistory/.bash_history" \
    -a "export PYENV_ROOT='/opt/pyenv'" \
    -a "export PATH='\$PYENV_ROOT/bin:\$PATH'" \
    -a "eval \"\$(pyenv init -)\"" \
    -a "export PATH='/opt/cargo/bin:\$PATH'" \
    -a "export PATH='/usr/local/go/bin:/opt/go/bin:\$PATH'" \
    -x

# Copy built application from builder stage
COPY --from=builder --chown=$USERNAME:$USERNAME /app/node_modules /app/node_modules
COPY --from=builder --chown=$USERNAME:$USERNAME /app/packages/types/dist /app/packages/shared/dist
COPY --from=builder --chown=$USERNAME:$USERNAME /app/packages/types/package.json /app/packages/shared/package.json
COPY --from=builder --chown=$USERNAME:$USERNAME /app/packages/types/node_modules /app/packages/shared/node_modules
COPY --from=builder --chown=$USERNAME:$USERNAME /app/apps/backend/dist /app/apps/backend/dist
COPY --from=builder --chown=$USERNAME:$USERNAME /app/apps/backend/package.json /app/apps/backend/package.json
COPY --from=builder --chown=$USERNAME:$USERNAME /app/apps/backend/node_modules /app/apps/backend/node_modules

# Copy entrypoint script
USER root
COPY docker/docker-entrypoint.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

# Final setup
USER $USERNAME

# Environment variables
ENV DEVCONTAINER=true
ENV NODE_ENV=production
ENV WORKER_MODE=true
ENV DATA_DIR=/data
ENV WORKSPACE_DIR=/workspace
ENV PORT=8001
ENV TZ="UTC"

# Expose port
EXPOSE 8001

# Set entrypoint
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
