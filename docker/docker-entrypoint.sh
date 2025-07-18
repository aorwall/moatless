#!/bin/bash
set -e

# Initialize firewall rules if running with privileges
# Disabled by default - uncomment to enable firewall rules
# if [ -f /usr/local/bin/init-firewall.sh ]; then
#     sudo /usr/local/bin/init-firewall.sh || true
# fi

# Ensure workspace directory exists with correct permissions
if [ ! -d "$WORKSPACE_DIR" ]; then
    mkdir -p "$WORKSPACE_DIR"
fi

# Ensure the workspace directory is owned by ubuntu user
# This is needed when the workspace path is custom (not /repo)
if [ "$(stat -c %U "$WORKSPACE_DIR" 2>/dev/null || echo 'root')" != "ubuntu" ]; then
    echo "Fixing ownership of workspace directory: $WORKSPACE_DIR"
    sudo chown -R ubuntu:ubuntu "$WORKSPACE_DIR"
fi

# Ensure claude config directory exists with correct permissions
if [ ! -d "$CLAUDE_CONFIG_DIR" ]; then
    mkdir -p "$CLAUDE_CONFIG_DIR"
fi

# Configure git worktree environment if needed
if [ "$USE_GIT_WORKTREES" = "true" ] && [ -f "$WORKSPACE_DIR/.git" ]; then
    echo "Detected git worktree, configuring git environment..."
    
    # Read the gitdir path from the .git file
    GITDIR=$(cat "$WORKSPACE_DIR/.git" | sed 's/gitdir: //')
    
    # Extract the worktree name from the path
    WORKTREE_NAME=$(basename "$GITDIR")
    
    # Set git environment variables instead of modifying files
    export GIT_DIR="/main-repo-git/worktrees/$WORKTREE_NAME"
    export GIT_WORK_TREE="$WORKSPACE_DIR"
    
    echo "Configured git environment variables:"
    echo "  GIT_DIR=$GIT_DIR"
    echo "  GIT_WORK_TREE=$GIT_WORK_TREE"
fi

# Configure git with GitHub personal access token if provided
if [ -n "$GITHUB_TOKEN" ]; then
    echo "Configuring git with GitHub personal access token..."
    
    # Configure git to use the token for GitHub HTTPS operations
    git config --global url."https://${GITHUB_TOKEN}@github.com/".insteadOf "https://github.com/"
    
    # Configure git credential helper to store credentials
    git config --global credential.helper store
    
    echo "Git configured for GitHub access with personal access token."
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
                # Continue with other commands even if one fails
            fi
        fi
    done

    echo "Setup commands completed."
else
    echo "No setup commands to run."
fi


# Start the backend in worker mode
echo "Starting Moatless backend in worker mode..."
echo "Worker API URL: ${WORKER_API_URL}"
echo "Worker Task ID: ${WORKER_TASK_ID}"
echo "Repo Dir: $WORKSPACE_DIR"
echo "Port: $PORT"

# Start the moatless-vibe server using the globally installed npm package
exec /usr/bin/moatless-vibe start