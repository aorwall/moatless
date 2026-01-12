#!/bin/bash
set -e

# Ensure workspace directory exists
if [ ! -d "$WORKSPACE_DIR" ]; then
    mkdir -p "$WORKSPACE_DIR"
fi

# Ensure claude config directory exists
if [ ! -d "$CLAUDE_CONFIG_DIR" ]; then
    mkdir -p "$CLAUDE_CONFIG_DIR"
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

# Run setup commands if provided
if [ -n "$SETUP_COMMANDS" ]; then
    echo "Running setup commands in $WORKSPACE_DIR..."
    cd "$WORKSPACE_DIR" || {
        echo "ERROR: Cannot change to $WORKSPACE_DIR directory"
        exit 1
    }

    # Parse JSON array of setup commands and execute each one
    echo "$SETUP_COMMANDS" | jq -r '.[]' | while read -r command; do
        if [ -n "$command" ]; then
            echo "Running setup command: $command"
            if ! eval "$command"; then
                echo "WARNING: Setup command failed: $command"
            fi
        fi
    done

    echo "Setup commands completed."
else
    echo "No setup commands to run."
fi

# Start the sandbox application
echo "Starting Moatless Sandbox..."
echo "Workspace Dir: $WORKSPACE_DIR"
echo "Data Dir: ${DATA_DIR:-/opt/moatless/data}"
echo "Port: ${PORT:-8001}"

cd /opt/moatless/sandbox
exec bun run dist/index.js
