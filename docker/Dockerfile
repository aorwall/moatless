# moatless-universal: Comprehensive development environment
# Based on OpenAI codex-universal (MIT License)
# https://github.com/openai/codex-universal

# Stage 1: Base system with all development tools (rarely changes)
FROM ubuntu:24.04 AS base-system

ENV LANG="C.UTF-8"
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
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Install Node.js 22 via NodeSource repository (matches package.json requirement)
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
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

# Install git-delta
RUN ARCH=$(dpkg --print-architecture) && \
    wget "https://github.com/dandavison/delta/releases/download/0.18.2/git-delta_0.18.2_${ARCH}.deb" && \
    dpkg -i "git-delta_0.18.2_${ARCH}.deb" && \
    rm "git-delta_0.18.2_${ARCH}.deb"

# Install bun globally
ARG BUN_VERSION=1.2.18
ENV BUN_INSTALL=/opt/bun
ENV PATH="$BUN_INSTALL/bin:$PATH"

RUN ARCH=$(dpkg --print-architecture) && \
    BUN_ARCH=$([ "$ARCH" = "amd64" ] && echo "x64" || echo "aarch64") && \
    mkdir -p "$BUN_INSTALL/bin" \
    && curl -L --fail "https://github.com/oven-sh/bun/releases/download/bun-v${BUN_VERSION}/bun-linux-${BUN_ARCH}.zip" \
        -o /tmp/bun.zip \
    && unzip -q /tmp/bun.zip -d "$BUN_INSTALL/bin" \
    && mv "$BUN_INSTALL/bin/bun-linux-${BUN_ARCH}/bun" "$BUN_INSTALL/bin/bun" \
    && chmod +x "$BUN_INSTALL/bin/bun" \
    && rm -rf "$BUN_INSTALL/bin/bun-linux-${BUN_ARCH}" /tmp/bun.zip \
    && ln -s "$BUN_INSTALL/bin/bun" /usr/local/bin/bun \
    && echo 'export BUN_INSTALL=/opt/bun' >> /etc/profile \
    && echo 'export PATH="$BUN_INSTALL/bin:$PATH"' >> /etc/profile

# Install pnpm and yarn
RUN corepack enable && \
    corepack prepare pnpm@latest --activate && \
    corepack prepare yarn@stable --activate

# Install Claude Code CLI as root using bun
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

# Stage 2: Application layer (changes frequently)
FROM base-system AS final

# Copy entrypoint script (changes less often than app)
USER root
COPY docker/docker-entrypoint.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

# Copy and install the packaged moatless-vibe application
# This is the only layer that rebuilds when the application changes
COPY dist /tmp/moatless-vibe/dist
COPY bin /tmp/moatless-vibe/bin
COPY package.json README.md LICENSE /tmp/moatless-vibe/
# Remove the prepare and postinstall scripts that cause issues in Docker and install package
RUN cd /tmp/moatless-vibe && \
    node -e "const pkg = require('./package.json'); delete pkg.scripts.prepare; delete pkg.scripts.postinstall; require('fs').writeFileSync('package.json', JSON.stringify(pkg, null, 2))" && \
    npm pack && \
    npm install -g ./moatless-vibe-*.tgz && \
    rm -rf /tmp/moatless-vibe

# Create /data directory with proper permissions
RUN mkdir -p /data && \
    chown $USERNAME:$USERNAME /data

# Switch to non-root user for runtime
USER $USERNAME
WORKDIR /home/ubuntu

# Ensure npm global bin is in PATH for the ubuntu user
ENV PATH="/usr/bin:/usr/local/bin:$PATH"

# Environment variables
ENV DEVCONTAINER=true
ENV NODE_ENV=production
ENV WORKER_MODE=true
ENV WORKSPACE_DIR=/repo
ENV CLAUDE_CONFIG_DIR=/home/ubuntu/.claude
ENV PORT=8001
ENV TZ="UTC"

# Expose port
EXPOSE 8001

# Set entrypoint
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]