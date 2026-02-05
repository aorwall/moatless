#!/bin/bash
# Sync plugins from backend API before agent execution
# This script is called from docker-entrypoint-sandbox.sh
#
# Required environment variables:
# - REPO_CONFIG: JSON containing agentConfig.pluginPaths array
# - TASK_AUTH_TOKEN: JWT token for authenticating with backend API
# - BACKEND_API_URL: Base URL for backend API (e.g., http://backend:8000)
#
# Optional:
# - PLUGINS_DIR: Directory to sync plugins to (default: /opt/moatless/plugins)
#
# NOTE: This script intentionally does NOT use 'set -e' to ensure
# plugin download failures don't block sandbox startup.

PLUGINS_DIR="${PLUGINS_DIR:-/opt/moatless/plugins}"

# Check required environment variables
if [ -z "$TASK_AUTH_TOKEN" ]; then
    echo "Warning: TASK_AUTH_TOKEN not set, skipping plugin sync"
    exit 0
fi

if [ -z "$BACKEND_API_URL" ]; then
    echo "Warning: BACKEND_API_URL not set, skipping plugin sync"
    exit 0
fi

if [ -z "$REPO_CONFIG" ]; then
    echo "No REPO_CONFIG set, skipping plugin sync"
    exit 0
fi

# Ensure plugins directory exists
mkdir -p "$PLUGINS_DIR"

# Read plugin paths from REPO_CONFIG.agentConfig.pluginPaths
# These are relative paths like "marketplaces/repo-name/plugins/plugin-name"
PLUGIN_PATHS=$(echo "$REPO_CONFIG" | jq -r '.agentConfig.pluginPaths[]?' 2>/dev/null || true)

if [ -z "$PLUGIN_PATHS" ]; then
    echo "No plugins configured in REPO_CONFIG.agentConfig.pluginPaths"
    exit 0
fi

echo "Syncing plugins from backend API..."

# Process each plugin path
echo "$PLUGIN_PATHS" | while IFS= read -r rel_path; do
    [ -z "$rel_path" ] && continue

    # Extract plugin ID from relative path
    # Path format: marketplaces/{repo-name}/{plugin-path}
    # Plugin ID format: {plugin-name}@{marketplace-name}
    #
    # Examples:
    # - marketplaces/moatless-plugin-marketplace/plugins/moatless-ui -> moatless-ui@moatless-plugin-marketplace
    # - marketplaces/context7-plugin/. -> context7-plugin@context7-plugin (single-plugin repo)

    # Get marketplace name (second component)
    marketplace_name=$(echo "$rel_path" | cut -d'/' -f2)

    # Get plugin name (last component, or marketplace name if it's ".")
    plugin_source=$(echo "$rel_path" | cut -d'/' -f3-)
    if [ "$plugin_source" = "." ] || [ -z "$plugin_source" ]; then
        plugin_name="$marketplace_name"
    else
        plugin_name=$(basename "$plugin_source")
    fi

    plugin_id="${plugin_name}@${marketplace_name}"
    target_dir="$PLUGINS_DIR/$rel_path"

    # Check if plugin already exists (cached in workspace PVC)
    if [ -d "$target_dir" ] && [ -f "$target_dir/.synced" ]; then
        # Check version to see if we need to re-sync
        cached_version=$(cat "$target_dir/.synced" 2>/dev/null || echo "")

        # Get current version from backend
        current_version=$(curl -sSL \
            -H "X-Task-Token: $TASK_AUTH_TOKEN" \
            "${BACKEND_API_URL}/api/v1/plugins/${plugin_id}/version" 2>/dev/null | \
            jq -r '.version // empty' 2>/dev/null || echo "")

        if [ -n "$current_version" ] && [ "$cached_version" = "$current_version" ]; then
            echo "Plugin already synced (version match): $plugin_id"
            continue
        fi

        echo "Plugin version changed, re-syncing: $plugin_id"
        rm -rf "$target_dir"
    fi

    echo "Syncing plugin: $plugin_id -> $target_dir"
    mkdir -p "$target_dir"

    # Download and extract plugin TAR archive
    HTTP_CODE=$(curl -sSL -w "%{http_code}" \
        -H "X-Task-Token: $TASK_AUTH_TOKEN" \
        "${BACKEND_API_URL}/api/v1/plugins/${plugin_id}/download" \
        -o /tmp/plugin.tar 2>/dev/null || echo "000")

    if [ "$HTTP_CODE" != "200" ]; then
        echo "Warning: Failed to download plugin $plugin_id (HTTP $HTTP_CODE)"
        rm -f /tmp/plugin.tar
        rmdir "$target_dir" 2>/dev/null || true
        continue
    fi

    # Extract TAR archive
    if ! tar -xf /tmp/plugin.tar -C "$target_dir" 2>/dev/null; then
        echo "Warning: Failed to extract plugin $plugin_id"
        rm -f /tmp/plugin.tar
        rm -rf "$target_dir"
        continue
    fi
    rm -f /tmp/plugin.tar

    # Get and store version for cache validation
    version=$(curl -sSL \
        -H "X-Task-Token: $TASK_AUTH_TOKEN" \
        "${BACKEND_API_URL}/api/v1/plugins/${plugin_id}/version" 2>/dev/null | \
        jq -r '.version // empty' 2>/dev/null || echo "")

    if [ -n "$version" ]; then
        echo "$version" > "$target_dir/.synced"
    fi

    echo "Successfully synced plugin: $plugin_id"
done

echo "Plugin sync complete"

# Always exit successfully - plugin failures should not block sandbox startup
exit 0
