#!/bin/bash
set -e

# Ensure home directory exists for git config
mkdir -p "$HOME"

# Ensure workspace directory exists
if [ ! -d "$WORKSPACE_DIR" ]; then
    mkdir -p "$WORKSPACE_DIR"
fi

# Ensure claude config directory and debug log directory exist
# The Claude CLI writes debug logs to $CLAUDE_CONFIG_DIR/debug/<session>.txt
# and uses non-recursive mkdirSync, so we must pre-create the debug directory
if [ ! -d "$CLAUDE_CONFIG_DIR" ]; then
    mkdir -p "$CLAUDE_CONFIG_DIR"
fi
mkdir -p "$CLAUDE_CONFIG_DIR/debug"

# Point debug logs to /tmp which always exists (workaround for SDK lazy-init race condition)
export CLAUDE_CODE_DEBUG_LOGS_DIR="/tmp/claude-debug.log"

# Write Claude OAuth token to credentials file if provided via environment variable.
# CLI v2.1.27+ reads $CLAUDE_CONFIG_DIR/.credentials but does NOT read the
# CLAUDE_CODE_OAUTH_TOKEN env var directly (known issue anthropics/claude-code#8938).
# Also write the onboarding flag so the CLI skips interactive setup.
if [ -n "$CLAUDE_CODE_OAUTH_TOKEN" ]; then
    echo "Writing Claude OAuth credentials file..."
    cat > "$CLAUDE_CONFIG_DIR/.credentials" <<CRED
{"claudeAiOauth":{"accessToken":"$CLAUDE_CODE_OAUTH_TOKEN","refreshToken":"","expiresAt":"2099-01-01T00:00:00.000Z","scopes":["user:inference","user:profile"]}}
CRED
    chmod 600 "$CLAUDE_CONFIG_DIR/.credentials"
fi

# Ensure onboarding flag exists so CLI skips interactive setup in headless mode
if [ ! -f "$HOME/.claude.json" ]; then
    echo '{"hasCompletedOnboarding":true}' > "$HOME/.claude.json"
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

# Symlink server management scripts into /opt/moatless/bin (survives volume mount)
# The scripts are installed at /usr/local/bin/ in the Docker image
mkdir -p /opt/moatless/bin
if [ -f "/usr/local/bin/server" ] && [ ! -e "/opt/moatless/bin/server" ]; then
    ln -s /usr/local/bin/server /opt/moatless/bin/server
fi
if [ -f "/usr/local/bin/file-explorer" ] && [ ! -e "/opt/moatless/bin/file-explorer" ]; then
    ln -s /usr/local/bin/file-explorer /opt/moatless/bin/file-explorer
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

# Write server config files from REPO_CONFIG.servers array
# Each server gets a <name>-server.config.json file in DATA_DIR
if [ -n "$REPO_CONFIG" ]; then
    SERVERS_JSON=$(echo "$REPO_CONFIG" | jq -c '.servers // []' 2>/dev/null || echo "[]")
    SERVERS_COUNT=$(echo "$SERVERS_JSON" | jq 'length' 2>/dev/null || echo "0")

    if [ "$SERVERS_COUNT" -gt 0 ]; then
        echo "Writing server configs from REPO_CONFIG.servers ($SERVERS_COUNT servers)..."
        echo "$SERVERS_JSON" | jq -c '.[]' | while IFS= read -r srv; do
            SRV_NAME=$(echo "$srv" | jq -r '.name')
            SRV_PORT=$(echo "$srv" | jq -r '.port')
            SRV_COMMANDS=$(echo "$srv" | jq -c '.startCommands // []')
            SRV_TIMEOUT=$(echo "$srv" | jq -r '.startupTimeoutSeconds // 30')

            # Write config file
            jq -n \
              --argjson port "$SRV_PORT" \
              --argjson cmds "$SRV_COMMANDS" \
              --argjson timeout "$SRV_TIMEOUT" \
              --arg workspace "$WORKSPACE_DIR" \
              '{port: $port, startCommands: $cmds, startupTimeoutSeconds: $timeout, workspacePath: $workspace}' \
              > "$DATA_DIR/${SRV_NAME}-server.config.json"

            echo "  Wrote config: ${SRV_NAME}-server.config.json (port $SRV_PORT)"
        done
    else
        # Legacy: Write preview config from previewConfig if servers array not present
        echo "No servers array in REPO_CONFIG, checking previewConfig for backward compat..."
        echo "$REPO_CONFIG" | jq -e '.previewConfig' > /dev/null 2>&1 && {
            echo "$REPO_CONFIG" | jq '{
              port: (.previewConfig.port // 3000),
              startCommands: (.previewConfig.serverStartCommands // []),
              startupTimeoutSeconds: (.previewConfig.startupTimeoutSeconds // 120),
              workspacePath: env.WORKSPACE_DIR
            }' > "$DATA_DIR/preview-server.config.json" 2>/dev/null || true
            echo "  Wrote legacy config: preview-server.config.json"
        } || true
    fi
fi

# Write "starting" status for servers that have commands configured
# This allows frontend to show "starting" status during setup commands
if [ -n "$REPO_CONFIG" ]; then
    for config_file in "$DATA_DIR"/*-server.config.json; do
        [ -f "$config_file" ] || continue
        HAS_COMMANDS=$(jq -r '.startCommands // .serverCommands // [] | length' "$config_file" 2>/dev/null || echo "0")
        if [ "$HAS_COMMANDS" -gt 0 ]; then
            SRV_BASENAME=$(basename "$config_file" .config.json)
            echo '{"status":"starting","startedAt":"'$(date -u +%Y-%m-%dT%H:%M:%SZ)'"}' > "$DATA_DIR/${SRV_BASENAME}.status.json"
            echo "  Server status: starting (${SRV_BASENAME})"
        fi
    done
fi

# Start sandbox app FIRST (for backend health checks and monitor)
echo "Starting Moatless Sandbox..."
echo "Workspace Dir: $WORKSPACE_DIR"
echo "Data Dir: $DATA_DIR"

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

# Start all configured servers via the generic `server` command
if [ -n "$REPO_CONFIG" ]; then
    echo "$REPO_CONFIG" | jq -r '.servers[]?.name' 2>/dev/null | while IFS= read -r name; do
        if [ -n "$name" ]; then
            echo "Starting server: $name"
            /usr/local/bin/server "$name" start || {
                echo "Warning: Server '$name' start failed" >&2
            }
        fi
    done

    # Legacy fallback: start preview server if using previewConfig (no servers array)
    SERVERS_COUNT=$(echo "$REPO_CONFIG" | jq '.servers // [] | length' 2>/dev/null || echo "0")
    if [ "$SERVERS_COUNT" -eq 0 ] && [ -f "$DATA_DIR/preview-server.config.json" ]; then
        HAS_COMMANDS=$(jq -r '.startCommands // .serverCommands // [] | length' "$DATA_DIR/preview-server.config.json" 2>/dev/null || echo "0")
        if [ "$HAS_COMMANDS" -gt 0 ]; then
            echo "Starting preview server (legacy previewConfig)..."
            /usr/local/bin/server preview start || {
                echo "Warning: Preview server start failed" >&2
            }
        fi
    fi
fi

# Wait for the sandbox app process
wait $SANDBOX_PID
