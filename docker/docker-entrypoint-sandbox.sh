#!/bin/bash
set -e

# Minimal entrypoint — all initialization (credentials, git, servers) is now
# handled by the Rust sandbox binary's init module.

# Ensure basic directories exist (Rust init will create subdirs)
mkdir -p "$HOME"
mkdir -p "${WORKSPACE_DIR:-/workspace}"
mkdir -p "${DATA_DIR:-/opt/moatless/data}"
mkdir -p "${CLAUDE_CONFIG_DIR:-$HOME/.claude}/debug"

# Point debug logs to /tmp (workaround for SDK lazy-init race condition)
export CLAUDE_CODE_DEBUG_LOGS_DIR="/tmp/claude-debug.log"

# Configure git worktree environment if needed
if [ "$USE_GIT_WORKTREES" = "true" ] && [ -n "$GIT_WORKTREE_NAME" ]; then
    export GIT_DIR="/main-repo-git/worktrees/$GIT_WORKTREE_NAME"
    export GIT_WORK_TREE="$WORKSPACE_DIR"
fi

exec /usr/local/bin/sandbox
