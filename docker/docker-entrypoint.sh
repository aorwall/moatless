#!/bin/bash
set -e

# Initialize firewall rules if running with privileges
# Disabled by default - uncomment to enable firewall rules
# if [ -f /usr/local/bin/init-firewall.sh ]; then
#     sudo /usr/local/bin/init-firewall.sh || true
# fi

# Ensure data directory exists with correct permissions
if [ ! -d "$DATA_DIR" ]; then
    mkdir -p "$DATA_DIR"
fi

# Ensure workspace directory exists with correct permissions
if [ ! -d "$WORKSPACE_DIR" ]; then
    mkdir -p "$WORKSPACE_DIR"
fi

# Configure git worktree environment if needed
if [ "$USE_GIT_WORKTREES" = "true" ] && [ -f /repo/.git ]; then
    echo "Detected git worktree, configuring git environment..."
    
    # Read the gitdir path from the .git file
    GITDIR=$(cat /repo/.git | sed 's/gitdir: //')
    
    # Extract the worktree name from the path
    WORKTREE_NAME=$(basename "$GITDIR")
    
    # Set git environment variables instead of modifying files
    export GIT_DIR="/main-repo-git/worktrees/$WORKTREE_NAME"
    export GIT_WORK_TREE="/repo"
    
    echo "Configured git environment variables:"
    echo "  GIT_DIR=$GIT_DIR"
    echo "  GIT_WORK_TREE=$GIT_WORK_TREE"
fi

# Run setup commands if provided
if [ -n "$SETUP_COMMANDS" ]; then
    echo "Running setup commands in /repo..."
    cd /repo || {
        echo "ERROR: Cannot change to /repo directory"
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
echo "Worker API URL: ${WORKER_API_URL:-http://localhost:8000}"
echo "Worker Task ID: ${WORKER_TASK_ID:-not set}"
echo "Workspace Dir: $WORKSPACE_DIR"
echo "Data Dir: $DATA_DIR"
echo "Port: $PORT"

# Start the moatless-vibe server using the globally installed npm package
exec /usr/bin/moatless-vibe start