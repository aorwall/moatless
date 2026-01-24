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

# Configure git worktree environment if needed
if [ "$USE_GIT_WORKTREES" = "true" ] && [ -n "$GIT_WORKTREE_NAME" ]; then
    echo "Detected git worktree, configuring git environment..."

    export GIT_DIR="/main-repo-git/worktrees/$GIT_WORKTREE_NAME"
    export GIT_WORK_TREE="$WORKSPACE_DIR"

    echo "Configured git environment variables:"
    echo "  GIT_DIR=$GIT_DIR"
    echo "  GIT_WORK_TREE=$GIT_WORK_TREE"
fi

# Configure git with GitHub personal access token if provided
if [ -n "$GITHUB_TOKEN" ]; then
    echo "Configuring git with GitHub personal access token..."

    git config --global url."https://${GITHUB_TOKEN}@github.com/".insteadOf "https://github.com/"
    git config --global credential.helper store
    git config --global url."https://${GITHUB_TOKEN}@github.com/".insteadOf "git@github.com:"

    echo "Git configured for GitHub access with personal access token."
fi

# Configure git user name and email if provided
if [ -n "$GIT_USER_NAME" ]; then
    echo "Configuring git user.name: $GIT_USER_NAME"
    git config user.name "$GIT_USER_NAME"
fi

if [ -n "$GIT_USER_EMAIL" ]; then
    echo "Configuring git user.email: $GIT_USER_EMAIL"
    git config user.email "$GIT_USER_EMAIL"
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

# Start the sandbox application
echo "Starting Moatless Sandbox..."
echo "Workspace Dir: $WORKSPACE_DIR"
echo "Data Dir: $DATA_DIR"
echo "Port: ${PORT:-8001}"

cd /app/sandbox
bun run dist/index.js &
SANDBOX_PID=$!

# Run setup commands from REPO_CONFIG (if provided)
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
        cd /app/sandbox
    fi
fi

# Start preview server if configured
if [ -f "$DATA_DIR/preview-server.config.json" ]; then
    # Check if serverCommands is non-empty
    HAS_COMMANDS=$(jq -r '.serverCommands | length' "$DATA_DIR/preview-server.config.json" 2>/dev/null || echo "0")
    if [ "$HAS_COMMANDS" -gt 0 ]; then
        echo "Starting preview server..."
        /opt/moatless/bin/preview-server start || {
            echo "Warning: Preview server start failed (non-blocking)" >&2
        }
    fi
fi

# Wait for the sandbox app process
wait $SANDBOX_PID
