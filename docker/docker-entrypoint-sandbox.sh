#!/bin/bash
set -e

# Ensure workspace directory exists
# Note: No chown needed - container runs as moatless user, so mkdir creates with correct ownership
if [ ! -d "$WORKSPACE_DIR" ]; then
    mkdir -p "$WORKSPACE_DIR"
fi

# Ensure claude config directory exists
# Note: No chown needed - container runs as moatless user, so mkdir creates with correct ownership
if [ ! -d "$CLAUDE_CONFIG_DIR" ]; then
    mkdir -p "$CLAUDE_CONFIG_DIR"
fi

# Ensure data directory exists
DATA_DIR="${DATA_DIR:-/opt/moatless/data}"
if [ ! -d "$DATA_DIR" ]; then
    mkdir -p "$DATA_DIR"
fi

# Symlink skills directory to the baked-in skills location
# Skills are stored at /usr/share/moatless-skills to survive volume mounts
SKILLS_SOURCE="/usr/share/moatless-skills"
SKILLS_TARGET="$CLAUDE_CONFIG_DIR/skills"
if [ -d "$SKILLS_SOURCE" ] && [ ! -e "$SKILLS_TARGET" ]; then
    ln -s "$SKILLS_SOURCE" "$SKILLS_TARGET"
    echo "Linked skills directory: $SKILLS_TARGET -> $SKILLS_SOURCE"
fi

# Symlink preview-server script into /opt/moatless/bin (survives volume mount)
# The script is installed at /usr/local/bin/preview-server in the Docker image
if [ -f "/usr/local/bin/preview-server" ] && [ ! -e "/opt/moatless/bin/preview-server" ]; then
    mkdir -p /opt/moatless/bin
    ln -s /usr/local/bin/preview-server /opt/moatless/bin/preview-server
fi

# Symlink task-metadata script into /opt/moatless/bin (survives volume mount)
# The script is installed at /usr/local/bin/task-metadata in the Docker image
if [ -f "/usr/local/bin/task-metadata" ] && [ ! -e "/opt/moatless/bin/task-metadata" ]; then
    mkdir -p /opt/moatless/bin
    ln -s /usr/local/bin/task-metadata /opt/moatless/bin/task-metadata
fi

# Configure git worktree environment if needed
if [ "$USE_GIT_WORKTREES" = "true" ] && [ -n "$GIT_WORKTREE_NAME" ]; then
    echo "Detected git worktree, configuring git environment..."

    export GIT_DIR="/main-repo-git/worktrees/$GIT_WORKTREE_NAME"
    export GIT_WORK_TREE="$WORKSPACE_DIR"

    echo "Configured git environment variables:"
    echo "  GIT_DIR=$GIT_DIR"
    echo "  GIT_WORK_TREE=$GIT_WORK_TREE"
fi

# Configure git with GitHub token via gh CLI credential helper
if [ -n "$GITHUB_TOKEN" ]; then
    echo "Configuring git and gh CLI with GitHub token..."

    # Configure gh CLI to use git credential helper
    # This makes git use gh CLI for authentication
    gh auth setup-git

    # SSH to HTTPS conversion (for repos using git@ URLs)
    git config --global url."https://github.com/".insteadOf "git@github.com:"

    echo "Git configured to use gh CLI for GitHub authentication."
fi

# Auto-configure git user from GitHub API (if not explicitly set)
if [ -n "$GITHUB_TOKEN" ] && { [ -z "$GIT_USER_NAME" ] || [ -z "$GIT_USER_EMAIL" ]; }; then
    echo "Fetching git user info from GitHub..."
    GH_USER_JSON=$(gh api user 2>/dev/null || true)
    if [ -n "$GH_USER_JSON" ]; then
        if [ -z "$GIT_USER_NAME" ]; then
            GIT_USER_NAME=$(echo "$GH_USER_JSON" | jq -r '.name // .login')
        fi
        if [ -z "$GIT_USER_EMAIL" ]; then
            GIT_USER_EMAIL=$(echo "$GH_USER_JSON" | jq -r '.email // empty')
            # Fallback to GitHub noreply email if user email is private
            if [ -z "$GIT_USER_EMAIL" ]; then
                GH_LOGIN=$(echo "$GH_USER_JSON" | jq -r '.login')
                GIT_USER_EMAIL="${GH_LOGIN}@users.noreply.github.com"
            fi
        fi
    fi
fi

# Configure git user name and email if provided
if [ -n "$GIT_USER_NAME" ]; then
    echo "Configuring git user.name: $GIT_USER_NAME"
    git config --global user.name "$GIT_USER_NAME"
fi

if [ -n "$GIT_USER_EMAIL" ]; then
    echo "Configuring git user.email: $GIT_USER_EMAIL"
    git config --global user.email "$GIT_USER_EMAIL"
fi

# Clone git repository if configured (blocking - needed for everything)
if [ -n "$GIT_CLONE_URL" ] && [ ! -d "$WORKSPACE_DIR/.git" ]; then
    echo "Cloning repository: $GIT_CLONE_URL"

    # Extract owner/repo from URL for gh CLI
    REPO_PATH=$(echo "$GIT_CLONE_URL" | sed -n 's|.*github\.com[/:]||p' | sed 's|\.git$||')

    BRANCH_ARG=""
    if [ -n "$GIT_BRANCH" ]; then
        BRANCH_ARG="--branch $GIT_BRANCH"
    fi

    gh repo clone "$REPO_PATH" "$WORKSPACE_DIR" -- --depth 1 $BRANCH_ARG
    echo "Repository cloned successfully"
fi

# Write preview config file from REPO_CONFIG env var
if [ -n "$REPO_CONFIG" ]; then
    echo "Writing preview server config from REPO_CONFIG..."
    echo "$REPO_CONFIG" | jq '{
      serverCommands: (.previewConfig.serverStartCommands // []),
      port: (.previewConfig.port // 3000),
      startupTimeoutSeconds: (.previewConfig.startupTimeoutSeconds // 120),
      workspacePath: env.WORKSPACE_DIR
    }' > "$DATA_DIR/preview-server.config.json" 2>/dev/null || true
fi

# Write "starting" status immediately if preview server is configured
# This allows frontend to show "starting" status during setup commands
if [ -f "$DATA_DIR/preview-server.config.json" ]; then
    HAS_COMMANDS=$(jq -r '.serverCommands | length' "$DATA_DIR/preview-server.config.json" 2>/dev/null || echo "0")
    if [ "$HAS_COMMANDS" -gt 0 ]; then
        echo '{"status":"starting","startedAt":"'$(date -u +%Y-%m-%dT%H:%M:%SZ)'"}' > "$DATA_DIR/preview-server.status.json"
        echo "Preview server status: starting (will start after setup commands)"
    fi
fi

# Start sandbox app FIRST (for backend health checks and monitor)
echo "Starting Moatless Sandbox..."
echo "Workspace Dir: $WORKSPACE_DIR"
echo "Data Dir: $DATA_DIR"
echo "Port: ${PORT:-8001}"

cd /app/sandbox
bun run dist/index.js &
SANDBOX_PID=$!

# Give sandbox a moment to start before running setup commands
sleep 1

# Run setup commands from REPO_CONFIG (sandbox is running during this)
if [ -n "$REPO_CONFIG" ]; then
    SETUP_COMMANDS=$(echo "$REPO_CONFIG" | jq -r '.codeExecution.setupCommands // [] | .[]' 2>/dev/null || true)
    if [ -n "$SETUP_COMMANDS" ]; then
        echo "Running setup commands..."
        cd "$WORKSPACE_DIR"
        echo "$SETUP_COMMANDS" | while IFS= read -r cmd; do
            if [ -n "$cmd" ]; then
                echo "  Running: $cmd"
                eval "$cmd" || {
                    echo "  Warning: setup command failed: $cmd" >&2
                }
            fi
        done
    fi
fi

# Start preview server LAST (will update status to "started" when health check passes)
if [ -f "$DATA_DIR/preview-server.config.json" ]; then
    HAS_COMMANDS=$(jq -r '.serverCommands | length' "$DATA_DIR/preview-server.config.json" 2>/dev/null || echo "0")
    if [ "$HAS_COMMANDS" -gt 0 ]; then
        echo "Starting preview server..."
        /opt/moatless/bin/preview-server start || {
            echo "Warning: Preview server start failed" >&2
        }
    fi
fi

# Wait for the sandbox app process
wait $SANDBOX_PID
