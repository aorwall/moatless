#!/bin/bash
set -e

# Validate worker mode - this image is for workers only
if [ "$WORKER_MODE" != "true" ]; then
    echo "ERROR: This image is for worker mode only (WORKER_MODE must be 'true')."
    echo "For the main web application, use the moatless-vibe image instead."
    exit 1
fi

# Initialize firewall rules if running with privileges
# Disabled by default - uncomment to enable firewall rules
# if [ -f /usr/local/bin/init-firewall.sh ]; then
#     sudo /usr/local/bin/init-firewall.sh || true
# fi

# Ensure workspace directory exists
# Note: In Kubernetes, fsGroup handles directory permissions via pod security context
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
    
    # Use the worktree name passed from the Docker environment setup
    # instead of reading from the .git file which contains host paths
    export GIT_DIR="/main-repo-git/worktrees/$GIT_WORKTREE_NAME"
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
    
    # Configure git to rewrite SSH URLs to HTTPS with token
    # This only affects git operations inside the container, not the actual remote configuration
    git config --global url."https://${GITHUB_TOKEN}@github.com/".insteadOf "git@github.com:"
    
    echo "Git configured to use HTTPS with token for all GitHub SSH URLs (container only)"
    
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
echo "Worker Task ID: ${TASK_ID}"
echo "Repo Dir: $WORKSPACE_DIR"
echo "Port: $PORT"

# Start the moatless-vibe server using the globally installed npm package
exec /usr/bin/moatless-vibe start