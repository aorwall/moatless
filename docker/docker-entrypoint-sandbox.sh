#!/bin/bash
set -e

# Ensure workspace directory exists with proper ownership
if [ ! -d "$WORKSPACE_DIR" ]; then
    mkdir -p "$WORKSPACE_DIR"
    chown -R moatless:moatless "$WORKSPACE_DIR"
fi

# Ensure claude config directory exists with proper ownership
if [ ! -d "$CLAUDE_CONFIG_DIR" ]; then
    mkdir -p "$CLAUDE_CONFIG_DIR"
    chown -R moatless:moatless "$CLAUDE_CONFIG_DIR"
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

# Note: Setup commands are NOT run here in the entrypoint
# They will be run by the sandbox application AFTER git clone completes
# Running them here would create files in an empty workspace directory,
# causing "directory is not empty" errors during git clone

# Start the sandbox application
echo "Starting Moatless Sandbox..."
echo "Workspace Dir: $WORKSPACE_DIR"
echo "Data Dir: ${DATA_DIR:-/opt/moatless/data}"
echo "Port: ${PORT:-8001}"

cd /opt/moatless/sandbox
exec bun run dist/index.js
